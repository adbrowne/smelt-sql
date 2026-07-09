//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-04` (`docs/research/20260705-property-discovery-loop.md` §4;
//! `docs/plans/20260705-property-discovery-loop.md` phase F).
//!
//! Same source shape as `SC-2` (`events(d, id, val)`, `mutation_profile:
//! mutable`), but the combiner is `MIN` — an idempotent monoid (Link 0 table
//! §2.0) that is nonetheless **non-invertible**: a stateful fold
//! `state = MIN(state, delta)` can only ever lower the running minimum, and
//! can never recover once the row that held the minimum is mutated upward
//! (there is no way to "un-fold" the old minimum back out). `SC-2` already
//! established that smelt's batched per-partition materialization is
//! unconditionally recompute-region (`DELETE [start,end)` + fresh `INSERT`
//! from current source contents), never a stateful fold onto remembered
//! state, for any combiner — so the non-invertibility hazard this cell
//! targets should not reproduce as long as the partition is explicitly
//! re-run (backfilled). Predicted (per the catalog hypothesis) REFUTED in the
//! "unsound fold" sense; this test asks empirically whether smelt's actual
//! emitted maintenance recovers the true (current) MIN after the min-holding
//! row is first lowered then raised, via an explicit backfill of the SAME
//! window — mirroring `SC-2`'s reachability discipline (the mutation must
//! land BETWEEN runs, never pre-populated, or both paths see it and the test
//! is vacuous).

use std::path::Path;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};
use smelt_maintenance_testkit::model_shapes::{idempotent_agg_mutable_source, ModelShape};

fn stage_project(shape: &ModelShape, project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    std::fs::write(
        project_dir.join(format!("models/{}.sql", shape.name)),
        shape.sql,
    )
    .unwrap();

    let cols: String = shape
        .source_columns
        .iter()
        .map(|c| format!("  - name: {}\n    type: {}\n", c.name, c.ty))
        .collect();
    // Declares `mutation_profile: mutable` — same world-fact as SC-2.
    let source_yml = format!(
        "description: property-discovery source.\nmutation_profile: mutable_snapshot\ncolumns:\n{cols}"
    );
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", shape.source)),
        source_yml,
    )
    .unwrap();

    let smelt_yml = format!(
        "name: property_discovery\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_events(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events AS
        SELECT d, id, CAST(val AS DOUBLE) AS val FROM (VALUES
            (DATE '2024-01-01', 1, 10.0),
            (DATE '2024-01-01', 2, 5.0)
        ) AS t(d, id, val);
        "#,
    )
    .expect("seed source");
}

/// Independent full-refresh oracle: `MIN(val)` over the CURRENT full contents
/// of the source table — no smelt compilation, no derived filter ("full
/// refresh over the source state at step k", design N3).
fn full_refresh_min(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT MIN(val) FROM main.sources_events WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("full-refresh oracle query")
}

fn maintained_min(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT min_val FROM main.events_daily_min_mutable WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

#[tokio::test]
async fn in_place_update_lowering_then_raising_the_min_is_recovered_on_backfill() {
    let shape = idempotent_agg_mutable_source();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_events(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Run 1: process [2024-01-01, 2024-01-02) — seeded rows (10.0, 5.0), min=5.0.
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
            maintained_min(&conn, "2024-01-01"),
            5.0,
            "run 1 must materialize the seeded min"
        );
    }

    // Between runs: LOWER the min-holding row (id=2) further — a stateful
    // fold `state = MIN(state, delta)` would track this new, lower value.
    {
        let conn = project.connect().expect("connect for lowering update");
        conn.execute("UPDATE main.sources_events SET val = 1.0 WHERE id = 2", [])
            .expect("in-place lowering update");
    }

    // Then RAISE that same row above every other row in the partition — the
    // non-invertible case: a fold that only ever lowered its running state to
    // 1.0 can never recover; only a fresh recompute over current source
    // contents notices the true minimum is now 10.0 (id=1).
    {
        let conn = project.connect().expect("connect for raising update");
        conn.execute(
            "UPDATE main.sources_events SET val = 999.0 WHERE id = 2",
            [],
        )
        .expect("in-place raising update");
    }

    // Run 2: advance the window FORWARD past the mutated partition — a plain
    // forward-only run never re-requests 2024-01-01, so (as in SC-2) it is
    // expected, not a bug, that this partition stays untouched. Establishes
    // the stale baseline the backfill run below is compared against.
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
        maintained_min(&conn, "2024-01-01")
    };
    assert_eq!(
        stale_after_forward_advance, 5.0,
        "expected (not a bug): a forward-only advance that never re-requests \
         2024-01-01 leaves that partition's materialized min untouched"
    );

    // Run 3: explicitly re-run (backfill) the SAME window run 1 processed —
    // the actual G-04 question: does the recompute-region backfill recover
    // the TRUE current min (10.0, from id=1), or does an idempotent-monoid
    // fold-style implementation get stuck at the lowest value ever observed
    // (1.0, from id=2's intermediate lowering)?
    let reporter3 = SqlCapturingReporter::new();
    project
        .run("run-3", request, &reporter3)
        .await
        .expect("run 3 (backfill) must succeed");

    let conn = project.connect().expect("connect after run 3");
    let maintained = maintained_min(&conn, "2024-01-01");
    let full_refresh = full_refresh_min(&conn, "2024-01-01");

    let compiled_sql = reporter3.sql_for(shape.name);
    assert!(
        !compiled_sql.is_empty(),
        "expected at least one compiled batch for {}",
        shape.name
    );

    assert_eq!(
        maintained, full_refresh,
        "smelt's maintained min ({maintained}) for the 2024-01-01 partition diverges from \
         the full-refresh oracle ({full_refresh}) after an explicit backfill re-run of that \
         SAME window, following an in-place update that first lowered then raised the \
         min-holding row's value between runs. Compiled SQL for the backfill run: {compiled_sql:?}"
    );
    assert_eq!(
        full_refresh, 10.0,
        "oracle sanity: after id=2 is raised to 999.0, id=1's 10.0 must be the true current \
         minimum (test setup bug otherwise)"
    );
}
