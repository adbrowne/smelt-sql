#![cfg(feature = "duckdb")]
//! End-to-end coverage for `smelt backbuild` dispatching cumulative_aggregate
//! models through the per-partition merge loop.
//!
//! Spec oracle:
//! - `docs/specs/cumulative_aggregate.md` §CLI: backbuild dispatches the
//!   per-partition merge loop so earlier partitions are not dropped.
//! - `docs/specs/cli.md` §"`smelt run` vs `smelt backbuild`": backbuild
//!   traverses upstream of the selector target(s).
//! - `docs/specs/architecture.md` §"Run pipeline parity rule": backbuild
//!   consumes `execute_project`; no compile/execute logic in `commands/backbuild.rs`.
//!
//! These tests stage hermetic TempDir workspaces and invoke the compiled
//! `smelt` binary (same discipline as `cumulative_classifier_gate.rs`).
//!
//! Fixture design: the driving-source model (`events`) is a `table` model with
//! `timeseries:` frontmatter and a self-contained VALUES literal — no external
//! seed or DuckDB pre-population needed, and the smelt classifier can resolve
//! the driving-source relationship via `smelt.events`. The cumulative model
//! references `smelt.events` in its FROM clause.

use std::path::{Path, PathBuf};
use std::process::Command;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_workspace(tmp: &Path, files: &[(&str, &str)]) {
    for (rel, contents) in files {
        let path = tmp.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&path, contents).expect("write workspace file");
    }
}

/// Read (device_id, event_count) from device_stats ordered by device_id.
fn read_device_stats(db_path: &Path) -> Vec<(i32, i64)> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb for reading");
    let mut stmt = conn
        .prepare("SELECT device_id, event_count FROM main.device_stats ORDER BY device_id")
        .expect("prepare");
    stmt.query_map([], |row| Ok((row.get::<_, i32>(0)?, row.get::<_, i64>(1)?)))
        .expect("execute query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows")
}

fn run_backbuild(
    project_dir: &Path,
    db_path: &Path,
    selector: &str,
    start: &str,
    end: &str,
) -> (bool, String) {
    let output = Command::new(smelt_bin())
        .args([
            "backbuild",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--database",
            db_path.to_str().unwrap(),
            "--start",
            start,
            "--end",
            end,
            selector,
        ])
        .env("RUST_LOG", "warn")
        .output()
        .expect("spawn smelt backbuild");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), combined)
}

/// `smelt backbuild` on a `cumulative_aggregate` model must dispatch the
/// per-partition merge loop, not a full-refresh.
///
/// **Red assertion**: with the legacy backbuild path, running backbuild for
/// [2026-01-02, 2026-01-03) performs a full-refresh over the Jan-2 window only,
/// dropping the Jan-1 partition from the cumulative table. So device_1's
/// event_count becomes 1 (Jan-2 only) rather than 3 (Jan-1:2 + Jan-2:1).
///
/// **Green assertion**: with `execute_project`, backbuild dispatches the
/// per-partition merge. Device_1's accumulated count across both windows = 3.
///
/// Fixture design: `events` is a self-contained VALUES table model with
/// `timeseries:` frontmatter (same pattern as `examples/cumulative_classifier_gate`).
/// `device_stats` is a `cumulative_aggregate` model referencing `smelt.events`
/// — the smelt classifier can resolve the driving source from the `smelt.*`
/// reference.
#[test]
fn backbuild_dispatches_cumulative_per_partition_merge() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    // The driving-source model: a self-contained VALUES table with two-day
    // data and timeseries frontmatter so the cumulative classifier can resolve
    // the driving source.
    let events_sql = r#"---
materialization: table
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT * FROM (
    VALUES
        (DATE '2026-01-01', 1),
        (DATE '2026-01-01', 1),
        (DATE '2026-01-02', 1),
        (DATE '2026-01-02', 2)
) AS t(event_date, device_id)
"#;

    // The cumulative_aggregate model: references smelt.events (the table model
    // above) so the classifier can derive the driving-source relationship.
    let device_stats_sql = r#"---
materialization: cumulative_aggregate
---
SELECT
    device_id,
    COUNT(*) AS event_count
FROM smelt.events
GROUP BY device_id
"#;

    let smelt_yml = format!(
        r#"name: backbuild_cumulative_e2e
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: {}
    schema: main
default_materialization: table
"#,
        db_path.display()
    );

    write_workspace(
        proj,
        &[
            ("smelt.yml", &smelt_yml),
            ("models/events.sql", events_sql),
            ("models/device_stats.sql", device_stats_sql),
        ],
    );

    // First backbuild: [2026-01-01, 2026-01-02) — Jan-1 only.
    // This creates the cumulative table with Jan-1 aggregate:
    //   device_1: count=2
    let (ok1, out1) = run_backbuild(proj, &db_path, "device_stats", "2026-01-01", "2026-01-02");
    assert!(
        ok1,
        "First backbuild (Jan-1) must succeed; output:\n{}",
        out1
    );

    let after_jan1 = read_device_stats(&db_path);
    // After Jan-1 window: device 1 has 2 events
    let dev1_after_jan1 = after_jan1
        .iter()
        .find(|(d, _)| *d == 1)
        .map(|(_, c)| *c)
        .expect("device_1 must be present after Jan-1 backbuild");
    assert_eq!(
        dev1_after_jan1, 2,
        "After Jan-1 backbuild, device_1 event_count must be 2 (Jan-1 has 2 rows for device_1)"
    );

    // Second backbuild: [2026-01-02, 2026-01-03) — Jan-2 only.
    // Per-partition merge must ADD Jan-2 delta (device_1:+1, device_2:+1)
    // to the existing cumulative, not replace it.
    let (ok2, out2) = run_backbuild(proj, &db_path, "device_stats", "2026-01-02", "2026-01-03");
    assert!(
        ok2,
        "Second backbuild (Jan-2) must succeed; output:\n{}",
        out2
    );

    let after_jan2 = read_device_stats(&db_path);

    // Device_1: Jan-1 contributed 2 events, Jan-2 contributed 1 event → total 3.
    // A full-refresh over Jan-2 only would give device_1: count=1 (loses Jan-1).
    let dev1_after_jan2 = after_jan2
        .iter()
        .find(|(d, _)| *d == 1)
        .map(|(_, c)| *c)
        .expect("device_1 must be present after Jan-2 backbuild");

    assert!(
        dev1_after_jan2 > 1,
        "device_1 event_count must be > 1 after Jan-2 backbuild — \
         the Jan-1 partition must survive (full-refresh would give 1, merge gives 3); \
         got: {}",
        dev1_after_jan2
    );

    // Device_2 was only in Jan-2; it must now appear with count=1.
    let dev2_count = after_jan2
        .iter()
        .find(|(d, _)| *d == 2)
        .map(|(_, c)| *c)
        .expect("device_2 must be present after Jan-2 backbuild");
    assert_eq!(
        dev2_count, 1,
        "device_2 only appeared in Jan-2, so event_count must be 1; got: {}",
        dev2_count
    );
}

/// `smelt backbuild --dry-run` must exit 0 and not materialise any tables.
///
/// Behavioural guard for the Phase 2 executor-path deletion: the legacy path
/// owned dry-run printing, so its removal needs proof that `execute_project`'s
/// dry-run branch is the active code path and writes nothing to the database.
#[test]
fn backbuild_dry_run_reports_plan_without_executing() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let events_sql = r#"---
materialization: table
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT * FROM (VALUES (DATE '2026-01-01', 1)) AS t(event_date, device_id)
"#;

    let device_stats_sql = r#"---
materialization: cumulative_aggregate
---
SELECT device_id, COUNT(*) AS event_count
FROM smelt.events
GROUP BY device_id
"#;

    let smelt_yml = format!(
        r#"name: backbuild_dry_run_e2e
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: {}
    schema: main
default_materialization: table
"#,
        db_path.display()
    );

    write_workspace(
        proj,
        &[
            ("smelt.yml", &smelt_yml),
            ("models/events.sql", events_sql),
            ("models/device_stats.sql", device_stats_sql),
        ],
    );

    let output = Command::new(smelt_bin())
        .args([
            "backbuild",
            "--project-dir",
            proj.to_str().unwrap(),
            "--database",
            db_path.to_str().unwrap(),
            "--dry-run",
            "--start",
            "2026-01-01",
            "--end",
            "2026-01-02",
            "device_stats",
        ])
        .env("RUST_LOG", "warn")
        .output()
        .expect("spawn smelt backbuild --dry-run");

    let combined = {
        let mut s = String::from_utf8_lossy(&output.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&output.stderr));
        s
    };

    assert!(
        output.status.success(),
        "backbuild --dry-run must exit 0; output:\n{}",
        combined
    );

    // Dry-run must not materialise any tables.
    let conn = duckdb::Connection::open(&db_path).expect("open duckdb after dry-run");
    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM information_schema.tables \
             WHERE table_schema = 'main' AND table_name = 'device_stats'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    assert!(
        !table_exists,
        "dry-run must not create device_stats table; found it in main schema"
    );
}

/// `smelt backbuild` targeting a downstream model must also rebuild upstream
/// models — the upstream-closure selector rewrite must be applied.
///
/// This test must pass both before and after the migration (it guards the
/// selector-rewrite behaviour, which was already present in the legacy path).
///
/// Fixture: `staging` is a plain `table` model with a VALUES literal; `device_summary`
/// is a cumulative_aggregate over `smelt.staging`. Backbuild selects only
/// `device_summary` — the upstream `staging` table must also be materialised.
#[test]
fn backbuild_traverses_upstream_closure() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    // `staging` is a plain table model with inline data and timeseries frontmatter
    // so the cumulative classifier can derive the driving-source relationship.
    let staging_sql = r#"---
materialization: table
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT * FROM (
    VALUES
        (DATE '2026-01-01', 10),
        (DATE '2026-01-02', 20)
) AS t(event_date, amount)
"#;

    // `device_summary` is a cumulative_aggregate over `smelt.staging`.
    // GROUP BY must not include the partition column (event_date), so we use
    // a synthetic `bucket` column derived from the amount to avoid that constraint.
    let device_summary_sql = r#"---
materialization: cumulative_aggregate
---
SELECT
    amount AS bucket,
    COUNT(*) AS row_count
FROM smelt.staging
GROUP BY amount
"#;

    let smelt_yml = format!(
        r#"name: backbuild_upstream_closure_e2e
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: {}
    schema: main
default_materialization: table
"#,
        db_path.display()
    );

    write_workspace(
        proj,
        &[
            ("smelt.yml", &smelt_yml),
            ("models/staging.sql", staging_sql),
            ("models/device_summary.sql", device_summary_sql),
        ],
    );

    // Backbuild targeting only `device_summary` — upstream `staging` must
    // also be materialised for the downstream to have data.
    let (ok, out) = run_backbuild(proj, &db_path, "device_summary", "2026-01-01", "2026-01-03");
    assert!(
        ok,
        "backbuild device_summary must succeed (upstream staging rebuilt); output:\n{}",
        out
    );

    // `staging` must exist and contain rows (upstream was rebuilt).
    let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM main.staging", [], |r| r.get(0))
        .expect("count staging rows");
    assert!(
        count > 0,
        "main.staging must contain rows after backbuild device_summary \
         — upstream closure must have rebuilt it; got count={}",
        count
    );

    // `device_summary` must also exist and have data.
    let summary_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM main.device_summary", [], |r| r.get(0))
        .expect("count device_summary rows");
    assert!(
        summary_count > 0,
        "main.device_summary must have rows after backbuild; got count={}",
        summary_count
    );
}
