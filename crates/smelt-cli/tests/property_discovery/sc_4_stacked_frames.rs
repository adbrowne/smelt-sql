//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `SC-4` (`docs/research/20260707-property-bounded-reach.md` §5/§7;
//! hypothesis B2 in `docs/research/property-discovery/catalog.jsonl`,
//! independently corroborated by
//! `docs/research/20260707-property-filter-distributivity.md` §7).
//!
//! Hypothesis: reach composes in SERIES along a nested path — a 7-day `RANGE`
//! frame inside a CTE consumed by a 3-day `RANGE` frame outside has true
//! backward reach 10 days — but the whole-text bound derivation max-merges
//! every frame it finds and derives 7 days. The widened source scan then
//! excludes rows 8–10 days back that a full refresh folds in, so the
//! maintained partition silently diverges. Expected RED = under-widened scan
//! (the model is ADMITTED — both frames are bounded `RANGE INTERVAL`, the
//! Form-A exemption — and produces wrong data); GREEN = the scan widens to
//! the series sum and the maintained value matches the full-refresh oracle.

use std::path::Path;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::model_shapes::{stacked_range_frames, ModelShape};

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
    let source_yml = format!(
        "description: property-discovery source.\nmutation_profile: append_only\ncolumns:\n{cols}timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n"
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

/// Seed shape: a large value 10 days before the final partition, an
/// intermediate row 3 days before it (whose 7-day window reaches the large
/// value), and the final partition's own row. `m3(2024-01-11)` must see
/// `s7(2024-01-08) = 100 + 1 = 101` — reachable only through the series sum
/// (the large row is 10 days back, outside the 7-day max-merged scan).
fn seed_sources(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_metrics AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 100.0),
            (DATE '2024-01-08', 1.0),
            (DATE '2024-01-11', 1.0)
        ) AS t(d, v);
        "#,
    )
    .expect("seed sources");
}

/// Independent full-refresh oracle: the model's own two-layer window logic
/// re-expressed over the CURRENT full contents of the source table.
fn full_refresh_m3_for(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!(
            "WITH seven AS (
                 SELECT d, SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS s7
                 FROM main.sources_metrics
             )
             SELECT m3 FROM (
                 SELECT d, MAX(s7) OVER (ORDER BY d RANGE BETWEEN INTERVAL '3 days' PRECEDING AND CURRENT ROW) AS m3
                 FROM seven
             ) WHERE d = DATE '{date}'"
        ),
        [],
        |row| row.get(0),
    )
    .expect("full-refresh oracle query")
}

fn maintained_m3_for(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT m3 FROM main.metrics_stacked WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

/// SC-4's owning test. Run 1 processes the seeded history (correct — the
/// scan covers everything from empty state). Run 2 processes only the final
/// day's partition: its source scan is widened by the derived backward
/// reach, so an under-derived reach (max-merge = 7d instead of the series
/// sum = 10d) silently truncates the inner running sum and the maintained
/// `m3` diverges from the full-refresh oracle.
#[tokio::test]
async fn late_row_inside_summed_reach_is_folded() {
    let shape = stacked_range_frames();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_sources(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Run 1: [2024-01-01, 2024-01-11) — the whole seeded history.
    let mut request = base_request("dev");
    request.enforce_safety = true;
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-11".to_string());
    project
        .run_quiet("run-1", request)
        .await
        .expect("run 1 must be admitted (both frames are bounded RANGE INTERVAL)");

    // Run 2: [2024-01-11, 2024-01-12) — only the final partition. The scan
    // must reach back 10 days (7d inner + 3d outer) to rebuild s7(2024-01-08)
    // correctly for m3(2024-01-11).
    let mut request2 = base_request("dev");
    request2.enforce_safety = true;
    request2.start = Some("2024-01-11".to_string());
    request2.end = Some("2024-01-12".to_string());
    project
        .run_quiet("run-2", request2)
        .await
        .expect("run 2 must be admitted");

    let conn = project.connect().expect("connect after run 2");
    let maintained = maintained_m3_for(&conn, "2024-01-11");
    let oracle = full_refresh_m3_for(&conn, "2024-01-11");

    assert!(
        (maintained - oracle).abs() < 1e-9,
        "LATENT BUG (SC-4 confirmed): stacked RANGE frames (7d inside a CTE, 3d outside) \
         need a series-summed backward reach of 10d, but the derived scan bound \
         under-widened the source read — maintained m3(2024-01-11) = {maintained} where the \
         full-refresh oracle = {oracle}. The max-merged bound truncates the inner running \
         sum for rows near the scan edge."
    );
}
