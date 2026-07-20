//! W6·P3 — Cross-engine `read_parquet` substitution end-to-end.
//!
//! Proves that a DuckDB model referencing a Spark model resolves to a
//! `read_parquet(...)` expression **through the run pipeline** (not just in
//! unit tests), and that DuckDB can read the Spark model's materialized rows.
//!
//! With `SPARK_CONNECT_URL` unset or `--features spark` absent: skips green.
//! With both set: runs the full cross-engine exchange path.

// This entire file requires both spark and duckdb features; gate at the top so
// the compiler does not emit dead-code / unused-import warnings in other builds.
#![cfg(all(feature = "spark", feature = "duckdb"))]

mod common;
use common::spark_connect_url;
use std::path::PathBuf;
use tempfile::TempDir;

/// Dedicated Spark schema for this test (avoids conflicts with other tests).
const SPARK_SCHEMA: &str = "smelt_cross_p3";

/// Stage a two-model workspace: `spark_side` on the Spark target, `duckdb_side`
/// on the default DuckDB target referencing `spark_side` via `smelt.ref()`.
fn stage_cross_engine_workspace(tmp: &TempDir) -> PathBuf {
    let root = tmp.path().join("cross_engine_proj");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();

    let url = spark_connect_url().unwrap_or_else(|| "sc://localhost:15002".to_string());

    // Use the actual Spark server's warehouse (bind-mounted path accessible to the host).
    // Spark Connect ignores the smelt.yml warehouse field for writes — it always uses
    // the server's spark.sql.warehouse.dir. The smelt.yml warehouse tells DuckDB where
    // to read the Parquet; both must be the same bind-mounted path.
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let default_wh = repo_root.join(".smelt-spark-warehouse");
    let wh_path = std::env::var("SMELT_SPARK_WAREHOUSE")
        .map(std::path::PathBuf::from)
        .unwrap_or(default_wh);
    std::fs::create_dir_all(&wh_path).unwrap();
    let wh = wh_path
        .to_str()
        .expect("warehouse path is valid UTF-8")
        .to_string();

    let yml = format!(
        "name: cross_engine_proj\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n  spark:\n    type: spark\n    connect_url: {url}\n    catalog: spark_catalog\n    schema: {SPARK_SCHEMA}\n    warehouse: {wh}\n    format: parquet\ndefault_materialization: table\n"
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();

    // Spark model: materializes SELECT 42 AS answer to Parquet in the warehouse.
    std::fs::write(
        root.join("models").join("spark_side.sql"),
        "---\ntarget: spark\nmaterialization: table\n---\nSELECT CAST(42 AS BIGINT) AS answer\n",
    )
    .unwrap();

    // DuckDB model: references spark_side; must resolve to read_parquet(...) after wiring.
    std::fs::write(
        root.join("models").join("duckdb_side.sql"),
        "SELECT answer FROM smelt.spark_side\n",
    )
    .unwrap();

    root
}

/// Run `smelt run --target dev` on the given project and return (stdout, stderr, success).
fn smelt_run_dev(project_dir: &std::path::Path) -> (String, String, bool) {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"));
    cmd.args([
        "run",
        "--project-dir",
        project_dir.to_str().unwrap(),
        "--target",
        "dev",
    ]);
    if let Some(url) = spark_connect_url() {
        cmd.env("SPARK_CONNECT_URL", url);
    }
    let out = cmd.output().expect("failed to spawn `smelt run`");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

/// Prove that a DuckDB model referencing a Spark model reads the Spark rows via Parquet.
///
/// **Red** (before P3 wiring): DuckDB tries `SELECT answer FROM main.spark_side`
/// which does not exist → run fails.
/// **Green** (after P3 wiring): DuckDB SQL resolves to `read_parquet(...)` pointing
/// at the Spark Parquet output → run succeeds and DuckDB returns `[{answer: 42}]`.
#[test]
fn duckdb_reads_spark_model_via_parquet() {
    if !cfg!(feature = "spark") {
        eprintln!("spark feature not enabled — skipping duckdb_reads_spark_model_via_parquet");
        return;
    }
    let Some(_connect_url) = spark_connect_url() else {
        eprintln!("SPARK_CONNECT_URL unset — skipping duckdb_reads_spark_model_via_parquet");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let root = stage_cross_engine_workspace(&tmp);
    let db_path = root.join("target/dev.duckdb");

    // Run the full pipeline (Spark model → DuckDB model).
    let (stdout, stderr, success) = smelt_run_dev(&root);
    assert!(
        success,
        "smelt run failed — cross-engine ref not wired.\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Verify DuckDB received the row from the Spark model via Parquet.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let rows = rt.block_on(async {
        use smelt_backend::Backend;
        use smelt_backend_duckdb::DuckDbBackend;

        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("DuckDB open failed after run");
        let batches = backend
            .execute_sql("SELECT answer FROM main.duckdb_side ORDER BY answer")
            .await
            .expect("DuckDB query on duckdb_side failed");

        let mut vals: Vec<String> = Vec::new();
        for batch in &batches {
            let col = batch.column(0);
            use arrow::array::Array;
            for i in 0..col.len() {
                vals.push(arrow::util::display::array_value_to_string(col, i).unwrap_or_default());
            }
        }
        vals
    });

    assert_eq!(
        rows,
        vec!["42".to_string()],
        "DuckDB duckdb_side should have Spark model row [42], got: {rows:?}"
    );
}

/// W4·P2: `connect_url` built from an env-interpolated port (`${SMELT_SPARK_PORT}`)
/// resolves at config-load time and the run succeeds against the live server —
/// proving the CLI's config-load interpolation pass (not a Spark-only mechanism)
/// covers the Spark target's `connect_url` end to end.
#[test]
fn spark_target_via_interpolated_url() {
    if !cfg!(feature = "spark") {
        eprintln!("spark feature not enabled — skipping spark_target_via_interpolated_url");
        return;
    }
    let Some(_connect_url) = spark_connect_url() else {
        eprintln!("SPARK_CONNECT_URL unset — skipping spark_target_via_interpolated_url");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("interpolated_url_proj");
    std::fs::create_dir_all(root.join("models")).unwrap();

    let port = std::env::var("SMELT_SPARK_PORT").unwrap_or_else(|_| "15002".to_string());

    let yml = "name: interpolated_url_proj\nversion: 1\npaths:\n  - models\ntargets:\n  spark:\n    type: spark\n    connect_url: sc://localhost:${SMELT_SPARK_PORT}\n    catalog: spark_catalog\n    schema: smelt_cross_p2_interp\n    format: parquet\ndefault_materialization: table\n";
    std::fs::write(root.join("smelt.yml"), yml).unwrap();
    std::fs::write(
        root.join("models").join("interp_model.sql"),
        "---\ntarget: spark\nmaterialization: table\n---\nSELECT CAST(1 AS BIGINT) AS n\n",
    )
    .unwrap();

    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"));
    cmd.args([
        "run",
        "--project-dir",
        root.to_str().unwrap(),
        "--target",
        "spark",
    ]);
    cmd.env("SPARK_CONNECT_URL", spark_connect_url().unwrap());
    cmd.env("SMELT_SPARK_PORT", &port);
    let out = cmd.output().expect("failed to spawn `smelt run`");
    assert!(
        out.status.success(),
        "smelt run failed with an interpolated connect_url.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
