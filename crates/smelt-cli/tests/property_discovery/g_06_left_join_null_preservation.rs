//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-06` (`docs/research/20260705-property-discovery-loop.md` §4).
//!
//! Hypothesis: a left-join (fact `events` LEFT JOIN right-side `refunds`),
//! both declared append-only timeseries sources, emits a row with
//! `refund_amt IS NULL` when no matching `refunds` row exists yet at run
//! time. The matching `refunds` row then arrives **late** — appended
//! between runs into the SAME `d` partition `events` already processed
//! (never mutated in place: this is the `SC-1` "late append into an
//! already-processed range" shape, distinct from `G-04`/`G-05`'s in-place
//! UPDATE shape). The empirical question (paper §10): does smelt's
//! recompute-region (an explicit backfill re-run of that window) re-read
//! `refunds`'s CURRENT contents and replace the stranded NULL, or does the
//! left-join's unmatched-row footprint stay unbounded-forward and get
//! missed even on backfill?
//!
//! Reachability (design §2.1 coverage caveat, N4): the `refunds` row is
//! APPENDED between two `execute_project` runs, into a partition already
//! materialized by run 1 — a pre-populated `refunds` table would be visible
//! to both the incremental and full-refresh paths and mask the bug this
//! cell hunts. As in `SC-2`/`G-04`/`G-05`, run 2 is a forward-only advance
//! (expected, not a bug, to leave the already-processed partition's NULL
//! stale) and run 3 is an explicit backfill of the SAME window — the actual
//! G-06 question.

use std::path::Path;

use crate::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};
use crate::model_shapes::{left_join_late_right_side, MultiSourceModelShape};
use crate::oracle::multiset_equal;

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
        // Both sources stay append-only (the default) — no `mutation_profile:`
        // block. This cell is about a late APPEND, not an in-place mutation.
        let source_yml =
            format!("description: property-discovery source.\ncolumns:\n{cols}{ts_block}");
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
        CREATE OR REPLACE TABLE main.sources_refunds (
            refund_date DATE, user_id BIGINT, refund_amt DOUBLE
        );
        "#,
    )
    .expect("seed sources");
}

/// Full-refresh oracle: the model's own left-join, re-expressed over the
/// CURRENT full contents of both staged source tables — no smelt
/// compilation, no derived filter ("full-refresh over the source state at
/// step k", design N3).
fn full_refresh_sql() -> &'static str {
    "SELECT e.d, e.user_id, e.val, r.refund_amt
     FROM main.sources_events e
     LEFT JOIN main.sources_refunds r ON e.user_id = r.user_id AND e.d = r.refund_date
     WHERE e.d = DATE '2024-01-01'"
}

fn maintained_sql() -> &'static str {
    "SELECT d, user_id, val, refund_amt FROM main.events_with_refunds WHERE d = DATE '2024-01-01'"
}

fn maintained_refund_amt(conn: &duckdb::Connection) -> Option<f64> {
    conn.query_row(
        "SELECT refund_amt FROM main.events_with_refunds WHERE d = DATE '2024-01-01' AND user_id = 1",
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

#[tokio::test]
async fn late_refund_appended_between_runs_is_recovered_on_backfill_but_not_forward_advance() {
    let shape = left_join_late_right_side();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_sources(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Run 1: process [2024-01-01, 2024-01-02) — no refund row exists yet.
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
            maintained_refund_amt(&conn),
            None,
            "run 1 must materialize the left-join NULL (no matching refund yet)"
        );
    }

    // Between runs: APPEND the matching refund row — a late arrival into the
    // ALREADY-PROCESSED 2024-01-01 partition. Not an in-place mutation: both
    // sources stay append-only.
    {
        let conn = project.connect().expect("connect for late refund append");
        conn.execute(
            "INSERT INTO main.sources_refunds VALUES (DATE '2024-01-01', 1, 3.5)",
            [],
        )
        .expect("late refund append");
    }

    // Run 2: advance the window FORWARD past the already-processed
    // partition — a plain forward-only run never re-requests 2024-01-01, so
    // it is expected (not a bug) to leave that partition's NULL stale.
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
        maintained_refund_amt(&conn)
    };
    assert_eq!(
        stale_after_forward_advance, None,
        "expected (not a bug): a forward-only advance that never re-requests \
         2024-01-01 leaves that partition's left-join NULL untouched"
    );

    // Run 3: explicitly re-run (backfill) the SAME window run 1 processed —
    // the actual G-06 question: does recompute-region re-read `refunds`'s
    // CURRENT contents, matching full-refresh?
    let reporter3 = SqlCapturingReporter::new();
    project
        .run("run-3", request, &reporter3)
        .await
        .expect("run 3 (backfill) must succeed");

    let conn = project.connect().expect("connect after run 3");
    let maintained_after_backfill = maintained_refund_amt(&conn);

    let compiled_sql = reporter3.sql_for(shape.name);
    assert!(
        !compiled_sql.is_empty(),
        "expected at least one compiled batch for {}",
        shape.name
    );

    assert!(
        multiset_equal(&conn, maintained_sql(), full_refresh_sql()),
        "smelt's maintained left-join for the 2024-01-01 partition diverges from the \
         full-refresh oracle after an explicit backfill re-run of that SAME window, \
         following a late refund append between runs. maintained refund_amt={maintained_after_backfill:?}. \
         Compiled SQL for the backfill run: {compiled_sql:?}"
    );
    assert_eq!(
        maintained_after_backfill,
        Some(3.5),
        "oracle sanity: after the late append, 3.5 must be the true current refund_amt \
         for user_id=1 (test setup bug otherwise)"
    );
}
