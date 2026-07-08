//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `SC-2` (`docs/research/20260705-property-discovery-loop.md` §4, §2.3;
//! `docs/plans/20260705-property-discovery-loop.md` phase E).
//!
//! Hypothesis: `input_delta.rs:88-93` classifies a clocked source as
//! `WindowForward` regardless of its declared `MutationProfile` — the
//! `Mutable` case falls through the same `_ if shape.has_clock` guard as
//! `AppendOnly`/undeclared. Predicted: an in-place `UPDATE` of a row inside an
//! already-processed partition, landing between two runs, is never picked up
//! by smelt's emitted batched maintenance even when the SAME window is
//! explicitly re-run (backfilled) afterwards — the maintained `total` for
//! that partition stays stale where full-refresh (over the source state at
//! the final step) reflects the mutated value.
//!
//! Reachability (design §2.1 coverage caveat, N4): the row is mutated
//! in-place directly against `main.sources_events` between two
//! `execute_project` runs — a pre-populated (post-mutation) source would be
//! visible to both the incremental and full-refresh paths and mask the bug
//! this cell hunts. Run 2 explicitly re-requests the SAME window run 1
//! already processed (a backfill), isolating "does re-deriving a partition
//! smelt already materialized re-read current source truth" from the
//! separate, expected limitation that a plain forward-only advance would
//! never revisit an already-closed window at all.

use std::path::Path;

use crate::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};
use crate::model_shapes::{additive_agg_mutable_source, ModelShape};

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
    // Declares `mutation_profile: mutable` — the world-fact the SC-2
    // hypothesis is about. Undeclared sources fail closed to SnapshotDiff
    // per `input_delta.rs`'s own doc comment; a clocked source must declare
    // `mutable` to reach the branch this cell targets.
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
            (DATE '2024-01-01', 1, 10.0)
        ) AS t(d, id, val);
        "#,
    )
    .expect("seed source");
}

/// Independent full-refresh oracle: the model's own `SUM` logic, re-expressed
/// as a plain query over the CURRENT full contents of the source table — no
/// smelt compilation, no derived filter ("full-refresh over the source state
/// at step k", design N3).
fn full_refresh_total(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT SUM(val) FROM main.sources_events WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("full-refresh oracle query")
}

fn maintained_total(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT total FROM main.events_daily_total WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

#[tokio::test]
async fn in_place_update_of_already_processed_partition_is_missed_on_forward_only_advance() {
    let shape = additive_agg_mutable_source();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_events(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Run 1: process [2024-01-01, 2024-01-02) — the only seeded row, val=10.0.
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
            maintained_total(&conn, "2024-01-01"),
            10.0,
            "run 1 must materialize the seeded row's total"
        );
    }

    // Between runs: mutate the seeded row IN PLACE — the already-processed
    // 2024-01-01 partition. This row was present (val=10.0) at run 1 and is
    // rewritten only now (design §2.1 reachability rule).
    {
        let conn = project.connect().expect("connect for mutation");
        conn.execute(
            "UPDATE main.sources_events SET val = 999.0 WHERE id = 1",
            [],
        )
        .expect("in-place update");
    }

    // Run 2: advance the window FORWARD past the mutated partition — a plain
    // forward-only run never re-requests 2024-01-01, so it is expected
    // (trivially, not a bug) to leave that partition untouched. This step
    // establishes the baseline "stale" state the backfill run below is
    // compared against.
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
        maintained_total(&conn, "2024-01-01")
    };
    assert_eq!(
        stale_after_forward_advance, 10.0,
        "expected (not a bug): a forward-only advance that never re-requests \
         2024-01-01 leaves that partition's materialized total untouched"
    );

    // Run 3: explicitly re-run (backfill) the SAME window run 1 processed.
    // This is the actual SC-2 question: does re-deriving a partition smelt
    // has already materialized re-read CURRENT source truth (recompute the
    // partition from the mutated row), or does the clocked-source ⇒
    // WindowForward classification (input_delta.rs:88-93, which does not
    // condition on MutationProfile::Mutable at all) cause smelt's emitted
    // maintenance to treat this partition as already-seen and skip re-deriving
    // it even under an explicit backfill request?
    let reporter3 = SqlCapturingReporter::new();
    project
        .run("run-3", request, &reporter3)
        .await
        .expect("run 3 (backfill) must succeed");

    let conn = project.connect().expect("connect after run 3");
    let maintained = maintained_total(&conn, "2024-01-01");
    let full_refresh = full_refresh_total(&conn, "2024-01-01");

    let compiled_sql = reporter3.sql_for(shape.name);
    assert!(
        !compiled_sql.is_empty(),
        "expected at least one compiled batch for {}",
        shape.name
    );

    assert_eq!(
        maintained, full_refresh,
        "LATENT BUG (SC-2 confirmed): smelt's maintained total ({maintained}) for the \
         2024-01-01 partition diverges from the full-refresh oracle ({full_refresh}) \
         after an explicit backfill re-run of that SAME window, following an in-place \
         mutation of the partition's only row between runs. Compiled SQL for the \
         backfill run: {compiled_sql:?}"
    );
    assert_eq!(
        full_refresh, 999.0,
        "oracle sanity: the mutated row's new value must dominate the full-refresh sum \
         (test setup bug otherwise)"
    );
}
