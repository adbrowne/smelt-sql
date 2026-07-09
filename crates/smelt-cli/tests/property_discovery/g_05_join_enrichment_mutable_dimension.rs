//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-05` (`docs/research/20260705-property-discovery-loop.md` §4).
//!
//! Hypothesis: an inner-join enrichment (fact `events` × dim `users`) over a
//! dim source declared `mutation_profile: mutable` broadcasts a dimension
//! update to every fact row referencing it (paper §10 — breaks invariant A,
//! keeps B). `users` carries no `timeseries:` block, so it never enters
//! `source_bounds`'s per-source margin derivation as a *driving* clocked
//! source; the empirical question is whether smelt's batched recompute-region
//! (established unconditional per `SC-2`/`G-04`: `DELETE [start,end)` + fresh
//! `INSERT` from CURRENT source contents) reads the dim table's CURRENT truth
//! when a partition is explicitly re-run (backfilled), or whether some
//! spurious bound (the `SC-1` "no-bound-found" fallback shape) clips it.
//!
//! Reachability (design §2.1 coverage caveat, N4): the dim row is mutated
//! in-place between two `execute_project` runs — a pre-populated (post-
//! mutation) dimension value would be visible to both paths and mask the
//! bug this cell hunts. As in `SC-2`/`G-04`, run 2 is a forward-only advance
//! (expected to leave the already-processed partition stale — not a bug)
//! and run 3 is an explicit backfill of the SAME window (the actual
//! question: does re-deriving that partition pick up the dimension's
//! current value?).

use std::path::Path;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};
use smelt_maintenance_testkit::model_shapes::{
    join_enrichment_mutable_dimension, MultiSourceModelShape,
};
use smelt_maintenance_testkit::oracle::multiset_equal;

fn stage_project(shape: &MultiSourceModelShape, project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    std::fs::write(
        project_dir.join(format!("models/{}.sql", shape.name)),
        shape.sql,
    )
    .unwrap();

    for src in shape.sources {
        let cols: String = src
            .columns
            .iter()
            .map(|c| format!("  - name: {}\n    type: {}\n", c.name, c.ty))
            .collect();
        let ts_block = match src.timeseries {
            Some((event_time_col, partition_col)) => format!(
                "timeseries:\n  event_time_column: {event_time_col}\n  partition_column: {partition_col}\n  granularity: day\n"
            ),
            None => String::new(),
        };
        // `users` is the mutable dimension this cell is about; `events` stays
        // undeclared (append-only default) — only the dim's mutability is
        // under test.
        let mutation_block = if src.name == "users" {
            "mutation_profile: mutable_snapshot\n"
        } else {
            ""
        };
        let source_yml = format!(
            "description: property-discovery source.\n{mutation_block}columns:\n{cols}{ts_block}"
        );
        std::fs::write(
            project_dir.join(format!("models/sources/{}.yml", src.name)),
            source_yml,
        )
        .unwrap();
    }

    let smelt_yml = format!(
        "name: property_discovery\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_sources(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1, 10.0)
        ) AS t(d, user_id, val);
        CREATE OR REPLACE TABLE main.sources_users AS
        SELECT * FROM (VALUES (1, 'bronze')) AS t(user_id, tier);
        "#,
    )
    .expect("seed sources");
}

/// Full-refresh oracle: the model's own join, re-expressed over the CURRENT
/// full contents of both staged source tables — no smelt compilation, no
/// derived filter ("full-refresh over the source state at step k", design
/// N3).
fn full_refresh_sql() -> &'static str {
    "SELECT e.d, e.user_id, e.val, u.tier
     FROM main.sources_events e
     JOIN main.sources_users u ON e.user_id = u.user_id
     WHERE e.d = DATE '2024-01-01'"
}

fn maintained_sql() -> &'static str {
    "SELECT d, user_id, val, tier FROM main.events_enriched WHERE d = DATE '2024-01-01'"
}

fn maintained_tier(conn: &duckdb::Connection) -> String {
    conn.query_row(
        "SELECT tier FROM main.events_enriched WHERE d = DATE '2024-01-01' AND user_id = 1",
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

#[tokio::test]
async fn dimension_update_between_runs_is_recovered_on_backfill_but_not_forward_advance() {
    let shape = join_enrichment_mutable_dimension();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_sources(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Run 1: process [2024-01-01, 2024-01-02) — seeded tier='bronze'.
    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    let reporter = SqlCapturingReporter::new();
    project
        .run("run-1", request.clone(), &reporter)
        .await
        .expect("run 1 must succeed");

    {
        let conn = project.connect().expect("connect after run 1");
        assert_eq!(
            maintained_tier(&conn),
            "bronze",
            "run 1 must materialize the seeded dimension value"
        );
    }

    // Between runs: mutate the dimension row IN PLACE — the already-processed
    // 2024-01-01 partition's enrichment now points at stale dimension data.
    {
        let conn = project.connect().expect("connect for dimension mutation");
        conn.execute(
            "UPDATE main.sources_users SET tier = 'gold' WHERE user_id = 1",
            [],
        )
        .expect("in-place dimension update");
    }

    // Run 2: advance the window FORWARD past the mutated partition — a plain
    // forward-only run never re-requests 2024-01-01, so it is expected
    // (not a bug) to leave that partition's enrichment stale. Baseline for
    // the backfill comparison below.
    let mut request2 = base_request("dev");
    request2.start = Some("2024-01-02".to_string());
    request2.end = Some("2024-01-03".to_string());
    let reporter2 = SqlCapturingReporter::new();
    project
        .run("run-2", request2, &reporter2)
        .await
        .expect("run 2 must succeed");

    let stale_after_forward_advance = {
        let conn = project.connect().expect("connect after run 2");
        maintained_tier(&conn)
    };
    assert_eq!(
        stale_after_forward_advance, "bronze",
        "expected (not a bug): a forward-only advance that never re-requests \
         2024-01-01 leaves that partition's enrichment untouched"
    );

    // Run 3: explicitly re-run (backfill) the SAME window run 1 processed —
    // the actual G-05 question: does the recompute-region re-read the dim
    // table's CURRENT contents, matching full-refresh?
    let reporter3 = SqlCapturingReporter::new();
    project
        .run("run-3", request, &reporter3)
        .await
        .expect("run 3 (backfill) must succeed");

    let conn = project.connect().expect("connect after run 3");
    let maintained_after_backfill = maintained_tier(&conn);

    let compiled_sql = reporter3.sql_for(shape.name);
    assert!(
        !compiled_sql.is_empty(),
        "expected at least one compiled batch for {}",
        shape.name
    );

    assert!(
        multiset_equal(&conn, maintained_sql(), full_refresh_sql()),
        "smelt's maintained enrichment for the 2024-01-01 partition diverges from the \
         full-refresh oracle after an explicit backfill re-run of that SAME window, \
         following an in-place dimension update between runs. maintained tier={maintained_after_backfill:?}. \
         Compiled SQL for the backfill run: {compiled_sql:?}"
    );
    assert_eq!(
        maintained_after_backfill, "gold",
        "oracle sanity: after the dimension update, 'gold' must be the true current tier \
         for user_id=1 (test setup bug otherwise)"
    );
}
