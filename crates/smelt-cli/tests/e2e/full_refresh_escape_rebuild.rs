#![cfg(feature = "duckdb")]
//! End-to-end coverage for the `schema_evolution: strategy: full_refresh`
//! atomicity escape (`docs/specs/definition_deltas.md` §"The atomicity
//! rule"): a model that opts out of `ALTER`-based evolution rebuilds under
//! its new definition when its schema changes, rather than falling through
//! to a non-atomic standalone backfill `UPDATE` issued against a schema
//! that may not carry the new column.
//!
//! Mirrors `schema_evolution_incremental.rs`'s harness, with the model
//! opting into `schema_evolution: strategy: full_refresh`. Unlike the
//! ALTER-in-place case, a full rebuild recomputes every row — so, unlike
//! that test, there is no NULL-for-old-rows expectation: every row (old and
//! new) carries the new column's value.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

const V1_MODEL: &str = "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\nschema_evolution:\n  strategy: full_refresh\ntimeseries:\n  event_time_column: event_date\n  partition_column: event_date\n  granularity: day\n---\nSELECT\n    CAST(event_date AS DATE) AS event_date,\n    user_id,\n    amount\nFROM smelt.sources.raw.payments\n";

const V2_MODEL: &str = "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\nschema_evolution:\n  strategy: full_refresh\ntimeseries:\n  event_time_column: event_date\n  partition_column: event_date\n  granularity: day\n---\nSELECT\n    CAST(event_date AS DATE) AS event_date,\n    user_id,\n    amount,\n    CASE WHEN amount >= 100 THEN TRUE END AS is_large\nFROM smelt.sources.raw.payments\n";

fn setup_workspace(dir: &Path) {
    std::fs::create_dir_all(dir.join("models/sources/raw")).unwrap();

    std::fs::write(
        dir.join("smelt.yml"),
        "name: evo-full-refresh-test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: test.duckdb\n    schema: main\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("models/sources/raw/payments.yml"),
        "name: raw.payments\ncolumns:\n  - name: event_date\n    type: VARCHAR\n  - name: user_id\n    type: INTEGER\n  - name: amount\n    type: INTEGER\n",
    )
    .unwrap();

    let conn = duckdb::Connection::open(dir.join("test.duckdb")).unwrap();
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS raw;\n         CREATE TABLE raw.payments AS\n         SELECT * FROM (VALUES\n             ('2024-01-01', 1, 50),\n             ('2024-01-01', 2, 150),\n             ('2024-01-02', 1, 200)\n         ) AS t(event_date, user_id, amount);",
    )
    .unwrap();
    drop(conn);

    std::fs::write(dir.join("models/payments.sql"), V1_MODEL).unwrap();
}

fn run_smelt(args: &[&str], dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .args(args)
        .arg("--project-dir")
        .arg(dir.to_str().unwrap())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt: {e}"))
}

/// Rows of `main.payments` as `(event_date, user_id, is_large)`, ordered.
fn payments_rows(dir: &Path) -> Vec<(String, i64, Option<bool>)> {
    let conn = duckdb::Connection::open(dir.join("test.duckdb"))
        .unwrap_or_else(|e| panic!("open test.duckdb: {e}"));
    let mut stmt = conn
        .prepare(
            "SELECT CAST(event_date AS VARCHAR), user_id, is_large \
             FROM main.payments ORDER BY event_date, user_id",
        )
        .unwrap_or_else(|e| panic!("prepare: {e}"));
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap_or_else(|e| panic!("query: {e}"));
    rows.map(|r| r.unwrap_or_else(|e| panic!("row: {e}")))
        .collect()
}

/// v1 build over both days establishes the baseline table; adding a column
/// under `schema_evolution: strategy: full_refresh` must rebuild the whole
/// table under the new definition — every row (not just the new day's)
/// carries the new column's real value, since a full refresh recomputes
/// everything rather than backfilling in place.
#[test]
fn added_column_under_full_refresh_strategy_rebuilds_every_row() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_workspace(dir);

    let build1 = run_smelt(
        &[
            "build",
            "--event-time-start",
            "2024-01-01",
            "--event-time-end",
            "2024-01-03",
        ],
        dir,
    );
    assert!(
        build1.status.success(),
        "v1 build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build1.stdout),
        String::from_utf8_lossy(&build1.stderr),
    );

    std::fs::write(dir.join("models/payments.sql"), V2_MODEL).unwrap();
    let build2 = run_smelt(
        &[
            "build",
            "--event-time-start",
            "2024-01-01",
            "--event-time-end",
            "2024-01-03",
        ],
        dir,
    );
    assert!(
        build2.status.success(),
        "v2 build (added column under full_refresh strategy) should rebuild, not fail:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build2.stdout),
        String::from_utf8_lossy(&build2.stderr),
    );

    // Every row — including day 1's, which an ALTER-in-place path would
    // have left NULL — carries the new column's real value: a full refresh
    // recomputed the whole table under the new definition, never a
    // standalone backfill UPDATE layered on top of the old one.
    let rows = payments_rows(dir);
    assert_eq!(
        rows,
        vec![
            ("2024-01-01".to_string(), 1, None),
            ("2024-01-01".to_string(), 2, Some(true)),
            ("2024-01-02".to_string(), 1, Some(true)),
        ],
        "a full rebuild must populate is_large for every row from the new \
         definition, not just rows written by this run's window",
    );
}
