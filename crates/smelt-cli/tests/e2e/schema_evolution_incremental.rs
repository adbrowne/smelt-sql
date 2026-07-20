#![cfg(feature = "duckdb")]
//! End-to-end add-column schema evolution on an *incremental* model,
//! through the real `smelt` binary.
//!
//! Regression: the schema-evolution gate in `execute_project` can only diff
//! against a stored deployed schema (`.smelt/targets/dev/schemas/<model>.json`), but
//! only the full-refresh execution branch ever saved one. An incremental
//! model therefore never acquired a baseline: `smelt diff` reported it as
//! "new" forever, and adding a column to its SELECT crashed the next run
//! with a DuckDB binder error ("table has N columns but N+1 values were
//! supplied") instead of the documented automatic `ALTER TABLE ... ADD
//! COLUMN` (docs-site/docs/guide/schema-evolution.md §Safe changes).
//!
//! This test drives the documented behavior end to end: build v1, assert a
//! baseline exists, add a nullable column, build again over a later window,
//! and assert old rows carry NULL while new rows are populated.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

const V1_MODEL: &str = "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\ntimeseries:\n  event_time_column: event_date\n  partition_column: event_date\n  granularity: day\n---\nSELECT\n    CAST(event_date AS DATE) AS event_date,\n    user_id,\n    amount\nFROM smelt.sources.raw.payments\n";

const V2_MODEL: &str = "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\ntimeseries:\n  event_time_column: event_date\n  partition_column: event_date\n  granularity: day\n---\nSELECT\n    CAST(event_date AS DATE) AS event_date,\n    user_id,\n    amount,\n    CASE WHEN amount >= 100 THEN TRUE END AS is_large\nFROM smelt.sources.raw.payments\n";

fn setup_workspace(dir: &Path) {
    std::fs::create_dir_all(dir.join("models/sources/raw")).unwrap();

    std::fs::write(
        dir.join("smelt.yml"),
        "name: evo-incremental-test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: test.duckdb\n    schema: main\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("models/sources/raw/payments.yml"),
        "name: raw.payments\ncolumns:\n  - name: event_date\n    type: VARCHAR\n  - name: user_id\n    type: INTEGER\n  - name: amount\n    type: INTEGER\n",
    )
    .unwrap();

    // Pre-create the raw source table the source YAML points at.
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

/// v1 build must persist a deployed-schema baseline for the incremental
/// model, and a v2 build with one added nullable column must ALTER the
/// table in place: old rows NULL, new rows populated.
#[test]
fn add_column_on_incremental_model_alters_in_place() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_workspace(dir);

    // v1: build day 1 only.
    let build1 = run_smelt(
        &[
            "build",
            "--event-time-start",
            "2024-01-01",
            "--event-time-end",
            "2024-01-02",
        ],
        dir,
    );
    assert!(
        build1.status.success(),
        "v1 build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build1.stdout),
        String::from_utf8_lossy(&build1.stderr),
    );

    let baseline = dir.join(".smelt/targets/dev/schemas/payments.json");
    assert!(
        baseline.exists(),
        "incremental model should persist a deployed-schema baseline after \
         its first successful build; .smelt/targets/dev/schemas contains: {:?}",
        std::fs::read_dir(dir.join(".smelt/targets/dev/schemas"))
            .map(|d| d
                .filter_map(|e| e.ok().map(|e| e.file_name()))
                .collect::<Vec<_>>())
            .unwrap_or_default(),
    );

    // v2: add a nullable derived column (no ELSE arm keeps it nullable so
    // the change classifies as a safe in-place ALTER), build day 2.
    std::fs::write(dir.join("models/payments.sql"), V2_MODEL).unwrap();
    let build2 = run_smelt(
        &[
            "build",
            "--event-time-start",
            "2024-01-02",
            "--event-time-end",
            "2024-01-03",
        ],
        dir,
    );
    assert!(
        build2.status.success(),
        "v2 build (added nullable column) should auto-ALTER, not fail:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build2.stdout),
        String::from_utf8_lossy(&build2.stderr),
    );

    // Old rows (day 1) keep NULL for the new column; new rows are populated.
    let rows = payments_rows(dir);
    assert_eq!(
        rows,
        vec![
            ("2024-01-01".to_string(), 1, None),
            ("2024-01-01".to_string(), 2, None),
            ("2024-01-02".to_string(), 1, Some(true)),
        ],
        "pre-migration rows should carry NULL for the added column and \
         post-migration rows should populate it",
    );
}
