#![cfg(feature = "duckdb")]
//! End-to-end coverage for the functions × incremental × timeseries seam
//! (D1 phase of the feature sweep, docs/plans/20260530-feature-sweep.md).
//!
//! Exercises the `examples/fn_incremental_ts` fixture end-to-end through
//! `smelt run --event-time-start/--event-time-end`. Verifies:
//!
//!   1. Function bodies (`is_peak_hour`, `hour_bucket`) are correctly expanded
//!      inside an incremental model — the compile+execute pipeline splices them
//!      as part of the WHERE clause and SELECT list respectively.
//!   2. The time filter (WHERE event_date >= start AND event_date < end) is
//!      injected at the outer query AFTER function splicing, scoping the output
//!      to the requested run window.
//!   3. Idempotence: re-running the same window produces the same rows
//!      (DELETE+INSERT is correct when functions are involved).
//!   4. Only peak-hour events (9am–6pm EXTRACT(HOUR) BETWEEN 9 AND 18) are
//!      counted — the function predicate actually filtered.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn workspace_template_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/fn_incremental_ts")
}

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Seed the events source table. The source YAML is at
/// `models/sources/events.yml` with no `name:` override, so the smelt
/// compiler maps `smelt.sources.events` → `main.sources_events`.
///
/// Data covers two days (2024-03-01, 2024-03-02):
///   - Peak-hour events   (HOUR = 12, i.e. 12:00): 4 events per day
///   - Off-peak events    (HOUR =  2, i.e. 02:00): 3 events per day
///
/// So `hour_bucket` = 'peak' rows: 4 per day; 'off-peak' rows: 3 per day.
fn seed_events(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS main;")?;
    conn.execute_batch(
        r#"CREATE OR REPLACE TABLE main.sources_events AS
           SELECT
               TIMESTAMP '2024-03-01 12:00:00' + INTERVAL (i) MINUTE AS event_ts,
               CAST('2024-03-01' AS DATE)                             AS event_date,
               CAST(i % 4 AS INTEGER)                                 AS user_id,
               CAST(i % 2 AS INTEGER)                                 AS event_id
           FROM range(4) AS t(i)
           UNION ALL
           SELECT
               TIMESTAMP '2024-03-01 02:00:00' + INTERVAL (i) MINUTE AS event_ts,
               CAST('2024-03-01' AS DATE)                             AS event_date,
               CAST(i % 3 AS INTEGER)                                 AS user_id,
               CAST(i % 2 AS INTEGER)                                 AS event_id
           FROM range(3) AS t(i)
           UNION ALL
           SELECT
               TIMESTAMP '2024-03-02 12:00:00' + INTERVAL (i) MINUTE AS event_ts,
               CAST('2024-03-02' AS DATE)                             AS event_date,
               CAST(i % 4 AS INTEGER)                                 AS user_id,
               CAST(i % 2 AS INTEGER)                                 AS event_id
           FROM range(4) AS t(i)
           UNION ALL
           SELECT
               TIMESTAMP '2024-03-02 02:00:00' + INTERVAL (i) MINUTE AS event_ts,
               CAST('2024-03-02' AS DATE)                             AS event_date,
               CAST(i % 3 AS INTEGER)                                 AS user_id,
               CAST(i % 2 AS INTEGER)                                 AS event_id
           FROM range(3) AS t(i);"#,
    )?;
    Ok(())
}

fn run_window(project_dir: &Path, db_path: &Path, start: &str, end: &str, label: &str) {
    let output = Command::new(smelt_bin())
        .args([
            "run",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--database",
            db_path.to_str().unwrap(),
            "--select",
            "daily_peak_events",
            "--event-time-start",
            start,
            "--event-time-end",
            end,
        ])
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` ({label}): {e}"));
    if !output.status.success() {
        panic!(
            "`smelt run` ({label}) failed (exit {:?});\nstderr:\n{}\nstdout:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        );
    }
}

/// Read the output rows as (event_date, bucket, event_count).
fn read_output(db_path: &Path) -> Vec<(String, String, i64)> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare(
            "SELECT CAST(event_date AS VARCHAR), bucket, event_count
             FROM main.daily_peak_events
             ORDER BY event_date, bucket",
        )
        .expect("prepare query");
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })
    .expect("query rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect rows")
}

#[test]
fn fn_incremental_ts_function_expansion_and_time_filter() {
    let tmp = TempDir::new().expect("create tempdir");
    let workspace = tmp.path().join("workspace");
    copy_dir_all(&workspace_template_dir(), &workspace).expect("copy example workspace");

    let db_path = tmp.path().join("fn_inc.duckdb");
    seed_events(&db_path).expect("seed events");

    // Run window: 2024-03-01 only (exclusive end → excludes 03-02).
    run_window(&workspace, &db_path, "2024-03-01", "2024-03-02", "run #1");

    let rows = read_output(&db_path);

    // Only 2024-03-01 rows — time filter injection works.
    let dates: Vec<_> = rows.iter().map(|(d, _, _)| d.as_str()).collect();
    assert!(
        dates.iter().all(|d| *d == "2024-03-01"),
        "time filter must exclude 2024-03-02; got dates: {dates:?}"
    );

    // Function predicate (is_peak_hour) filtered correctly: bucket = 'peak' only.
    let buckets: Vec<_> = rows.iter().map(|(_, b, _)| b.as_str()).collect();
    assert!(
        buckets.iter().all(|b| *b == "peak"),
        "is_peak_hour filter must keep only peak events; got buckets: {buckets:?}"
    );

    // Count = 4 peak events seeded per (date, bucket) pair.
    // 4 events across 4 users → 1 per user → GROUP BY event_date, bucket yields 1 group with 4.
    // (all have same bucket='peak'; all 4 are distinct user_id 0,1,2,3)
    let total_peak: i64 = rows.iter().map(|(_, _, c)| c).sum();
    assert_eq!(
        total_peak, 4,
        "peak event count for 2024-03-01 must be 4 (from seeded data)"
    );

    // Idempotence: re-run the same window; output must not change.
    run_window(
        &workspace,
        &db_path,
        "2024-03-01",
        "2024-03-02",
        "run #2 (idempotent)",
    );
    let rows2 = read_output(&db_path);
    assert_eq!(
        rows, rows2,
        "re-running the same window must produce identical output (DELETE+INSERT idempotence \
         with function expansion)"
    );
}
