//! W5·P2 — five directly-testable dialect lowerings executed on live Spark.
//!
//! Exercises five of the six false-flag lowerings that are reachable through
//! user-written smelt SQL models:
//! 1. `QUALIFY`        → subquery + outer `WHERE` rewrite
//! 2. `DATE 'YYYY-MM-DD'` → `DATE('YYYY-MM-DD')` function call
//! 3. `val::BIGINT`    → `CAST(val AS BIGINT)`
//! 4. Trailing comma   → stripped from `SELECT` list
//! 5. `table` mat.     → `DROP TABLE IF EXISTS` + `CREATE TABLE` (not `CREATE OR REPLACE`)
//!
//! The sixth lowering — `ARRAY[a,b] → ARRAY(a,b)` (`supports_array_literal`) —
//! is NOT reachable via user SQL because `ARRAY[...]` is a smelt meta-language
//! list construct (triggers `MetaListInScalarPosition` if used as a SELECT item).
//! That lowering is exercised only by compiler-generated SQL (e.g. function body
//! expansions) and is covered by the printer unit tests in `smelt-dialect`.
//! This gap is logged in §"Coverage gaps deferred" in the W5 plan.
//!
//! DuckDB always runs; Spark runs only when `SPARK_CONNECT_URL` is set
//! and the binary is compiled with `--features spark`.  When `SPARK_CONNECT_URL`
//! is absent the test passes covering DuckDB only.

mod common;
use common::{assert_table_parity, fetch_rows, spark_connect_url, targets_to_run, TargetKind};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Unique Spark schema for this test to avoid conflicts with other Spark tests.
const SPARK_SCHEMA: &str = "smelt_lower_p2";

/// Model SQL exercising five directly-testable Spark lowerings:
/// - Trailing comma after `rn` in SELECT list (lowering 4 — stripped for Spark)
/// - `DATE '2024-01-01'` literal (lowering 2 — DATE('2024-01-01') for Spark)
/// - `val::BIGINT` cast (lowering 3 — CAST(val AS BIGINT) for Spark)
/// - `QUALIFY rn = 1` (lowering 1 — subquery + outer WHERE for Spark)
/// - `table` materialization (lowering 5 — DROP IF EXISTS + CREATE for Spark)
///
/// The QUALIFY selects the highest-`val` row per `grp`, leaving two output rows.
const LOWERING_MODEL: &str = "\
SELECT
    grp,
    DATE '2024-01-01' AS event_date,
    val::BIGINT AS int_val,
    ROW_NUMBER() OVER (PARTITION BY grp ORDER BY val DESC) AS rn,
FROM (
    SELECT 'alpha' AS grp, 100 AS val
    UNION ALL SELECT 'beta', 200
    UNION ALL SELECT 'alpha', 50
) AS base
QUALIFY rn = 1";

fn stage_lowering_workspace(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let root = tmp.path().join("lowering_proj");
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::create_dir_all(&warehouse).unwrap();

    let url = spark_connect_url().unwrap_or_else(|| "sc://localhost:15002".to_string());
    let wh_str = warehouse
        .to_str()
        .expect("warehouse path must be valid UTF-8");

    let yml = format!(
        "name: lowering_proj\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n  \
         spark:\n    type: spark\n    connect_url: {url}\n    catalog: spark_catalog\n    schema: {SPARK_SCHEMA}\n    warehouse: {wh_str}\n    format: delta\n\
         default_materialization: table\n"
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();
    std::fs::write(
        root.join("models").join("lowering_showcase.sql"),
        LOWERING_MODEL,
    )
    .unwrap();

    (root, warehouse)
}

fn run_smelt_run(project_dir: &std::path::Path, target: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args([
            "run",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--target",
            target,
        ])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

/// Expected rows after `QUALIFY rn = 1` (highest-`val` row per group).
/// Columns in materialized order: grp, event_date, int_val, rn.
fn expected_rows() -> Vec<Vec<String>> {
    let mut rows = vec![
        vec![
            "alpha".to_string(),
            "2024-01-01".to_string(),
            "100".to_string(),
            "1".to_string(),
        ],
        vec![
            "beta".to_string(),
            "2024-01-01".to_string(),
            "200".to_string(),
            "1".to_string(),
        ],
    ];
    rows.sort();
    rows
}

/// Five directly-testable Spark dialect lowerings execute on both backends and
/// produce the same logical result.
///
/// Covers QUALIFY, DATE literal, `::cast`, trailing comma, and `table`
/// materialization (CREATE OR REPLACE → DROP+CREATE).  The sixth lowering
/// (`ARRAY[...]` → `ARRAY(...)`) is compiler-generated-SQL only; it is covered
/// by printer unit tests in `smelt-dialect` and logged as a coverage gap in
/// the W5 plan §"Coverage gaps deferred".
///
/// **Red on Spark** if the server rejects any lowered construct — fix the
/// printer, re-run to confirm green.
/// **DuckDB always green; Spark skips when `SPARK_CONNECT_URL` is unset.**
#[cfg(feature = "duckdb")]
#[test]
fn all_lowerings_execute_on_both_backends() {
    let tmp = TempDir::new().unwrap();
    let (root, warehouse) = stage_lowering_workspace(&tmp);
    let db_path = root.join("target/dev.duckdb");

    for kind in targets_to_run() {
        let (target_name, schema) = match &kind {
            TargetKind::DuckDb => ("dev", "main"),
            TargetKind::Spark => ("spark", SPARK_SCHEMA),
        };

        let out = run_smelt_run(&root, target_name);
        assert!(
            out.status.success(),
            "{target_name}: `smelt run` failed.\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );

        let actual = fetch_rows(&kind, &db_path, &warehouse, schema, "lowering_showcase");
        assert_table_parity(&actual, &expected_rows(), target_name);
    }
}
