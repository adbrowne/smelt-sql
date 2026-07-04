//! materialization parity (view / table) across backends.
//!
//! Exercises `view` and `table` materializations via `smelt run` on both
//! DuckDB and Spark.  Both must produce a queryable relation with the same
//! logical rows.
//!
//! `materialized_view` is not a storage-axis value: `materialization:
//! materialized_view` is rejected at config-parse time with a migration hint
//! pointing to `refresh: materialized_view` (see `docs/specs/models.md`
//! §"Materialization (storage) modes"). That rejection is covered below by
//! `materialized_view_storage_value_is_rejected`.
//!
//! With `SPARK_CONNECT_URL` unset: DuckDB only (Spark path skips green).
//! With `SPARK_CONNECT_URL` set AND `--features spark`: also covers Spark.

mod common;
use common::{assert_table_parity, fetch_rows, spark_connect_url, targets_to_run, TargetKind};
use std::process::Command;
use tempfile::TempDir;

const SPARK_SCHEMA: &str = "smelt_mat_p6";

const VIEW_MODEL: &str = r#"---
materialization: view
---
WITH data AS (
    SELECT CAST(1 AS BIGINT) AS id, 'alpha' AS label
    UNION ALL SELECT CAST(2 AS BIGINT), 'beta'
    UNION ALL SELECT CAST(3 AS BIGINT), 'gamma'
)
SELECT id, label FROM data
"#;

const TABLE_MODEL: &str = r#"---
materialization: table
---
WITH data AS (
    SELECT CAST(1 AS BIGINT) AS id, 'alpha' AS label
    UNION ALL SELECT CAST(2 AS BIGINT), 'beta'
    UNION ALL SELECT CAST(3 AS BIGINT), 'gamma'
)
SELECT id, label FROM data
"#;

fn expected_rows() -> Vec<Vec<String>> {
    let mut rows = vec![
        vec!["1".to_string(), "alpha".to_string()],
        vec!["2".to_string(), "beta".to_string()],
        vec!["3".to_string(), "gamma".to_string()],
    ];
    rows.sort();
    rows
}

fn stage_mat_workspace(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = tmp.path().join("mat_proj");
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::create_dir_all(&warehouse).unwrap();

    let url = spark_connect_url().unwrap_or_else(|| "sc://localhost:15002".to_string());
    let wh_str = warehouse
        .to_str()
        .expect("warehouse path must be valid UTF-8");

    let yml = format!(
        "name: mat_proj\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n  \
           spark:\n    type: spark\n    connect_url: {url}\n    catalog: spark_catalog\n    \
           schema: {SPARK_SCHEMA}\n    warehouse: {wh_str}\n    format: delta\n\
         default_materialization: table\n"
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();
    std::fs::write(root.join("models").join("view_model.sql"), VIEW_MODEL).unwrap();
    std::fs::write(root.join("models").join("table_model.sql"), TABLE_MODEL).unwrap();

    (root, warehouse)
}

fn run_smelt(project_dir: &std::path::Path, target: &str) -> std::process::Output {
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

/// `view` and `table` materializations produce the same queryable rows on
/// both backends.
///
/// **DuckDB always green; Spark skips when `SPARK_CONNECT_URL` is unset.**
#[cfg(feature = "duckdb")]
#[test]
fn view_and_table_materialize_consistently_on_both() {
    let tmp = TempDir::new().unwrap();
    let (root, warehouse) = stage_mat_workspace(&tmp);
    let db_path = root.join("target/dev.duckdb");

    let expected = expected_rows();
    let mut ref_view: Vec<Vec<String>> = Vec::new();
    let mut ref_table: Vec<Vec<String>> = Vec::new();

    for kind in targets_to_run() {
        let (target_name, schema) = match &kind {
            TargetKind::DuckDb => ("dev", "main"),
            TargetKind::Spark => ("spark", SPARK_SCHEMA),
        };

        let out = run_smelt(&root, target_name);
        assert!(
            out.status.success(),
            "{target_name}: `smelt run` failed.\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );

        // view model: stored SELECT, must be queryable.
        let view_rows = fetch_rows(&kind, &db_path, &warehouse, schema, "view_model");
        assert_table_parity(&view_rows, &expected, &format!("{target_name}:view_model"));
        if ref_view.is_empty() {
            ref_view = view_rows;
        } else {
            assert_table_parity(
                &view_rows,
                &ref_view,
                &format!("{target_name}:view_model cross-backend"),
            );
        }

        // table model: physical table.
        let table_rows = fetch_rows(&kind, &db_path, &warehouse, schema, "table_model");
        assert_table_parity(
            &table_rows,
            &expected,
            &format!("{target_name}:table_model"),
        );
        if ref_table.is_empty() {
            ref_table = table_rows;
        } else {
            assert_table_parity(
                &table_rows,
                &ref_table,
                &format!("{target_name}:table_model cross-backend"),
            );
        }
    }
}

/// `materialization: materialized_view` is not a storage-axis value — it is
/// rejected at config-parse time with a migration hint pointing to
/// `refresh: materialized_view` (the refresh-axis replacement).
#[cfg(feature = "duckdb")]
#[test]
fn materialized_view_storage_value_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("mv_reject_proj");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();

    let yml = "name: mv_reject_proj\n\
               version: 1\n\
               paths:\n  - models\n\
               targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n";
    std::fs::write(root.join("smelt.yml"), yml).unwrap();
    std::fs::write(
        root.join("models").join("mv_model.sql"),
        "---\nmaterialization: materialized_view\n---\nSELECT 1 AS id\n",
    )
    .unwrap();

    let out = run_smelt(&root, "dev");
    assert!(
        !out.status.success(),
        "`smelt run` should fail for a `materialization: materialized_view` model"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("refresh: materialized_view"),
        "expected migration hint pointing to `refresh: materialized_view`, got: {stderr}"
    );
}
