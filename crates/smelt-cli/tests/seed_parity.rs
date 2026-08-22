//! W5·P1 — seed / `load_table` end-to-end parity on live remote backends.
//!
//! Proves a CSV seed loads through the real CLI path (`smelt seed`) into the
//! DuckDB, Spark, and BigQuery backends, not just via the backend's
//! `load_table` directly.
//!
//! With `SPARK_CONNECT_URL` unset: DuckDB only (Spark path skips green).
//! With `SPARK_CONNECT_URL` set AND `--features spark`: also covers Spark.
//! With `SMELT_BQ_PROJECT`/`SMELT_BQ_ACCESS_TOKEN` set AND `--features
//! bigquery`: also covers BigQuery.

mod common;
use common::{
    assert_table_parity, bq_target_block, drop_bq_dataset, fetch_rows, spark_connect_url,
    targets_to_run, TargetKind,
};
use std::process::Command;
use tempfile::TempDir;

/// Unique Spark schema for this test to avoid conflicts with other Spark tests.
const SPARK_SCHEMA: &str = "smelt_seed_p1";

/// Scopes this suite's BigQuery dataset, for the same reason.
const BQ_LABEL: &str = "seed_p1";

/// Stage a minimal workspace with a single flat seed CSV (`p1_users.csv`).
fn stage_seed_workspace(tmp: &TempDir) -> std::path::PathBuf {
    let root = tmp.path().join("seed_proj");
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(root.join("seeds")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::create_dir_all(&warehouse).unwrap();

    let url = spark_connect_url().unwrap_or_else(|| "sc://localhost:15002".to_string());
    let wh_str = warehouse
        .to_str()
        .expect("warehouse path must be valid UTF-8");

    let bq_block = bq_target_block(BQ_LABEL);
    let yml = format!(
        "name: seed_parity_proj\nversion: 1\npaths:\n  - seeds\ntargets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n  spark:\n    type: spark\n    connect_url: {url}\n    catalog: spark_catalog\n    schema: {SPARK_SCHEMA}\n    warehouse: {wh_str}\n    format: delta\n{bq_block}default_materialization: table\n"
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();

    // Simple 3-row seed: int + varchar + varchar (no date, avoids cross-backend date formatting).
    std::fs::write(
        root.join("seeds").join("p1_users.csv"),
        "id,name,region\n1,Alice,US\n2,Bob,GB\n3,Carol,AU\n",
    )
    .unwrap();

    root
}

fn run_smelt_seed(project_dir: &std::path::Path, target: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args([
            "seed",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--target",
            target,
        ])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt seed`: {e}"))
}

/// Expected rows in the `p1_users` table after seeding from `p1_users.csv`.
///
/// Integer ids are displayed as decimal strings by `array_value_to_string`.
/// Rows pre-sorted for comparison.
fn expected_rows() -> Vec<Vec<String>> {
    let mut rows = vec![
        vec!["1".to_string(), "Alice".to_string(), "US".to_string()],
        vec!["2".to_string(), "Bob".to_string(), "GB".to_string()],
        vec!["3".to_string(), "Carol".to_string(), "AU".to_string()],
    ];
    rows.sort();
    rows
}

/// Prove a CSV seed loads into both DuckDB and Spark through the real CLI path.
///
/// Runs `smelt seed --target <name>` for each backend in `targets_to_run()`,
/// then calls `fetch_rows` + `assert_table_parity` against the expected CSV data.
///
/// **Red on Spark** if the real CLI seed path has a gap (e.g. `load_table` fails
/// on a remote Connect JVM).  DuckDB is always green; Spark skips when
/// `SPARK_CONNECT_URL` is unset.
#[cfg(feature = "duckdb")]
#[test]
fn seed_loads_into_both_backends() {
    let tmp = TempDir::new().unwrap();
    let warehouse = tmp.path().join("warehouse");
    let root = stage_seed_workspace(&tmp);
    // DuckDB file created by `smelt seed --target dev` at target/dev.duckdb.
    let db_path = root.join("target/dev.duckdb");

    for kind in targets_to_run(BQ_LABEL) {
        let (target_name, schema) = match &kind {
            TargetKind::DuckDb => ("dev", "main"),
            TargetKind::Spark => ("spark", SPARK_SCHEMA),
            TargetKind::BigQuery { dataset } => ("bq", dataset.as_str()),
        };

        let out = run_smelt_seed(&root, target_name);
        assert!(
            out.status.success(),
            "{target_name}: `smelt seed` failed.\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );

        let actual = fetch_rows(&kind, &db_path, &warehouse, schema, "p1_users");
        drop_bq_dataset(&kind);
        assert_table_parity(&actual, &expected_rows(), target_name);
    }
}
