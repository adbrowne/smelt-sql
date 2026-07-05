//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-09` (`docs/research/20260705-property-discovery-loop.md` §4).
//!
//! Hypothesis: `UNION ALL` of two independent append-only timeseries sources
//! (`events_a`, `events_b`), batched per partition (`unique_key: [d, id,
//! src]`). A multiset union has no combiner state to fold (Link 0) — the only
//! question is whether smelt's batched **recompute-region** (the DELETE+INSERT
//! technique `G-03`/`G-06`/`G-07` already established for `refresh: batched`)
//! re-reads BOTH arms' CURRENT contents on an explicit backfill of an
//! already-processed partition, or whether a per-arm bound-derivation gap
//! (`source_bounds` scanning only the first `FROM` clause, `SC-1`'s known
//! shape) strands a late row appended into ONE arm while the other recomputes
//! correctly.
//!
//! Reachability (design §2.1 N4): a row is appended into EACH arm separately,
//! between runs, into the SAME already-processed `d` partition — a
//! pre-populated union would be visible to both the incremental and
//! full-refresh paths and mask the bug. Run 2 (forward-only advance) is
//! expected to leave the partition stale; run 3 (explicit backfill of the
//! same window) is the actual G-09 question — and must recover BOTH late
//! rows, not just one.

use std::path::Path;

use crate::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};
use crate::model_shapes::{union_all_two_append_only, MultiSourceModelShape};
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
        // Both arms stay append-only (the default) — this cell is about a
        // late APPEND into each arm, not an in-place mutation.
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
        CREATE OR REPLACE TABLE main.sources_events_a AS
        SELECT d, id, CAST(val AS DOUBLE) AS val FROM (VALUES
            (DATE '2024-01-01', 1, 10.0)
        ) AS t(d, id, val);
        CREATE OR REPLACE TABLE main.sources_events_b AS
        SELECT d, id, CAST(val AS DOUBLE) AS val FROM (VALUES
            (DATE '2024-01-01', 100, 5.0)
        ) AS t(d, id, val);
        "#,
    )
    .expect("seed sources");
}

/// Full-refresh oracle: the model's own `UNION ALL`, re-expressed over the
/// CURRENT full contents of both staged source tables — no smelt
/// compilation, no derived filter ("full-refresh over the source state at
/// step k", design N3).
fn full_refresh_sql() -> &'static str {
    "SELECT d, id, val, 'a' AS src FROM main.sources_events_a WHERE d = DATE '2024-01-01'
     UNION ALL
     SELECT d, id, val, 'b' AS src FROM main.sources_events_b WHERE d = DATE '2024-01-01'"
}

fn maintained_sql() -> &'static str {
    "SELECT d, id, val, src FROM main.events_union_all WHERE d = DATE '2024-01-01'"
}

fn maintained_row_count(conn: &duckdb::Connection) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM main.events_union_all WHERE d = DATE '2024-01-01'",
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

#[tokio::test]
async fn late_rows_appended_into_both_union_arms_between_runs_are_recovered_on_backfill_but_not_forward_advance(
) {
    let shape = union_all_two_append_only();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_sources(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Run 1: process [2024-01-01, 2024-01-02) — one row per arm exists.
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
            maintained_row_count(&conn),
            2,
            "run 1 must materialize exactly the two rows present at that time (one per arm)"
        );
    }

    // Between runs: APPEND a late row into EACH arm — a late arrival into the
    // ALREADY-PROCESSED 2024-01-01 partition. Not an in-place mutation: both
    // arms stay append-only.
    {
        let conn = project.connect().expect("connect for late appends");
        conn.execute(
            "INSERT INTO main.sources_events_a VALUES (DATE '2024-01-01', 2, 20.0)",
            [],
        )
        .expect("late append into arm A");
        conn.execute(
            "INSERT INTO main.sources_events_b VALUES (DATE '2024-01-01', 200, 50.0)",
            [],
        )
        .expect("late append into arm B");
    }

    // Run 2: advance the window FORWARD past the already-processed
    // partition — a plain forward-only run never re-requests 2024-01-01, so
    // it is expected (not a bug) to leave that partition's row count stale.
    let mut request2 = base_request("dev");
    request2.start = Some("2024-01-02".to_string());
    request2.end = Some("2024-01-03".to_string());
    let reporter2 = SqlCapturingReporter::new();
    project
        .run("run-2", request2, &reporter2)
        .await
        .expect("run 2 must succeed");

    {
        let conn = project.connect().expect("connect after run 2");
        assert_eq!(
            maintained_row_count(&conn),
            2,
            "expected (not a bug): a forward-only advance that never re-requests \
             2024-01-01 leaves that partition's row count stale (2, not 4)"
        );
    }

    // Run 3: explicitly re-run (backfill) the SAME window run 1 processed —
    // the actual G-09 question: does recompute-region re-read BOTH union
    // arms' CURRENT contents, matching full-refresh?
    let reporter3 = SqlCapturingReporter::new();
    project
        .run("run-3", request, &reporter3)
        .await
        .expect("run 3 (backfill) must succeed");

    let conn = project.connect().expect("connect after run 3");
    let row_count_after_backfill = maintained_row_count(&conn);

    let compiled_sql = reporter3.sql_for(shape.name);
    assert!(
        !compiled_sql.is_empty(),
        "expected at least one compiled batch for {}",
        shape.name
    );

    assert!(
        multiset_equal(&conn, maintained_sql(), full_refresh_sql()),
        "smelt's maintained UNION ALL for the 2024-01-01 partition diverges from the \
         full-refresh oracle after an explicit backfill re-run of that SAME window, \
         following late appends into BOTH arms between runs. \
         row_count_after_backfill={row_count_after_backfill}. \
         Compiled SQL for the backfill run: {compiled_sql:?}"
    );
    assert_eq!(
        row_count_after_backfill, 4,
        "oracle sanity: after both late appends, 4 rows must be the true current \
         union contents for 2024-01-01 (test setup bug otherwise)"
    );
}
