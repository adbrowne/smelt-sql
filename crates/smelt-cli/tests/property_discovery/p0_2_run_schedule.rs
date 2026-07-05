//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `P0-2` acceptance test (design §3b; plan phase C, acceptance (i)).
//!
//! Proves the run-schedule driver's defining capability: a late row appended
//! into an already-processed event-time range, **between** two runs, changes
//! the step-`k` source snapshot at the step it lands — the snapshot taken
//! right after the first `AdvanceWindowAndRun` step does NOT yet contain the
//! late row, but a "pre-populated" snapshot (all rows present from the start,
//! as a harness that only advances the window and never mutates the source
//! would see) is indistinguishable from the final state. A step-k oracle that
//! can't tell these apart can't catch a lateness bug (design N3) — this test
//! is the proof the driver can.

use chrono::NaiveDate;

use crate::link_c_harness::LinkCProject;
use crate::model_shapes::{batched_passthrough, ModelShape};
use crate::run_schedule::{snapshot, EventRow, RunScheduleDriver, ScheduleStep};

fn stage_project(shape: &ModelShape, project_dir: &std::path::Path, db_path: &std::path::Path) {
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
    let source_yml = format!("description: property-discovery source.\ncolumns:\n{cols}");
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

fn seed_events(db_path: &std::path::Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1, 10.0),
            (DATE '2024-01-02', 2, 20.0)
        ) AS t(d, id, val);
        "#,
    )
    .expect("seed source");
}

#[tokio::test]
async fn step_k_snapshot_differs_from_pre_populated_after_late_append() {
    let shape = batched_passthrough();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_events(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");
    let driver = RunScheduleDriver {
        project: &project,
        target: "dev",
        source_table: "main.sources_events",
    };

    let day1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let day2 = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
    let day3 = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();

    // Step 0: run the first window [day1, day2) — the late row does not exist
    // yet. Step 1: append a row dated day1 — INSIDE the range step 0 just
    // processed, landing *after* that run already happened. Step 2: run the
    // second window [day2, day3).
    let late_row = EventRow {
        d: day1,
        id: 99,
        val: 99.0,
    };
    let schedule = crate::run_schedule::RunSchedule(vec![
        ScheduleStep::AdvanceWindowAndRun {
            start: day1,
            end: day2,
        },
        ScheduleStep::AppendLateRow(late_row.clone()),
        ScheduleStep::AdvanceWindowAndRun {
            start: day2,
            end: day3,
        },
    ]);

    let snapshots = driver.execute(&schedule).await;
    assert_eq!(snapshots.len(), 3, "one snapshot per schedule step");

    let step0_snapshot = &snapshots[0]; // right after the first run, before the late append
    let final_snapshot = &snapshots[2]; // after the late row landed

    assert!(
        !step0_snapshot.contains(&late_row),
        "the step-0 (post-first-run) snapshot must NOT contain a row appended \
         AFTER that step — otherwise the driver isn't really doing between-run \
         mutation, it's pre-population: {step0_snapshot:?}"
    );
    assert!(
        final_snapshot.contains(&late_row),
        "the final snapshot must contain the late row once appended: {final_snapshot:?}"
    );

    // The "pre-populated" alternative a window-only harness would have used —
    // seed all rows including the late one up front, then take one snapshot —
    // is exactly `final_snapshot`. Assert it's provably different from the
    // step-k snapshot the driver actually captured at step 0: this is the gap
    // a step-k oracle exists to close.
    assert_ne!(
        step0_snapshot, final_snapshot,
        "step-k snapshot and the pre-populated (all-rows-up-front) snapshot \
         must diverge, or a lateness bug at step 0 would be invisible"
    );

    // Direct DB read-back cross-check: the source table's live content right
    // now must match the final snapshot (proves `snapshot()` isn't stale).
    let conn = project.connect().expect("connect");
    let live = snapshot(&conn, "main.sources_events");
    assert_eq!(
        &live, final_snapshot,
        "live source content must match the last recorded snapshot"
    );
}
