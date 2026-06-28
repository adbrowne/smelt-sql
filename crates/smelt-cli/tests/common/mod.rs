//! Shared dual-target test utilities for W1+ harness tests.
//!
//! DuckDB always runs; Spark runs only when `SPARK_CONNECT_URL` is set.
//! Tests that call `targets_to_run()` are automatically skipped on the Spark
//! path when no server is provisioned — they still pass by covering DuckDB only.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Which execution target this harness is running against.
pub enum TargetKind {
    DuckDb,
    Spark,
}

/// Returns `SPARK_CONNECT_URL` from the environment, or `None` when absent.
pub fn spark_connect_url() -> Option<String> {
    std::env::var("SPARK_CONNECT_URL").ok()
}

/// Returns the targets to run this iteration.
///
/// Always includes `DuckDb`. Appends `Spark` only when BOTH hold:
/// - `SPARK_CONNECT_URL` is set (a live Connect server is reachable), AND
/// - the test binary was compiled with `--features spark` (so the `smelt`
///   binary also has Spark support).
///
/// This ensures `cargo test --quiet` (the default, no spark feature) is always
/// green even when `SPARK_CONNECT_URL` is present in the environment.
pub fn targets_to_run() -> Vec<TargetKind> {
    let mut targets = vec![TargetKind::DuckDb];
    if cfg!(feature = "spark") && spark_connect_url().is_some() {
        targets.push(TargetKind::Spark);
    }
    targets
}

/// Returns `(target_name, yaml_block)` for a single target kind.
///
/// `yaml_block` is the indented YAML content that goes under the target name
/// key (4-space indent — ready to embed inside `targets:` in smelt.yml).
pub fn targets_yaml(kind: &TargetKind, warehouse_dir: &Path) -> (String, String) {
    match kind {
        TargetKind::DuckDb => (
            "dev".to_string(),
            "type: duckdb\n    database: target/dev.duckdb\n    schema: main".to_string(),
        ),
        TargetKind::Spark => {
            let url = spark_connect_url().unwrap_or_else(|| "sc://localhost:15002".to_string());
            let warehouse = warehouse_dir
                .to_str()
                .expect("warehouse path must be valid UTF-8");
            (
                "spark".to_string(),
                format!(
                    "type: spark\n    connect_url: {url}\n    \
                     catalog: spark_catalog\n    schema: smelt_w1\n    \
                     warehouse: {warehouse}\n    format: delta"
                ),
            )
        }
    }
}

/// Stages a smelt workspace with both a `dev` (DuckDB) and `spark` target.
///
/// The `smelt.yml` always includes both target blocks so a test can run
/// `--target spark` when Spark is up. `targets_to_run()` controls which
/// targets are actually exercised in the loop.
pub fn stage_dual_workspace(
    tmp: &TempDir,
    name: &str,
    models: &[(&str, &str)],
    warehouse_dir: &Path,
) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::create_dir_all(warehouse_dir).unwrap();

    let (dev_name, dev_yaml) = targets_yaml(&TargetKind::DuckDb, warehouse_dir);
    let (spark_name, spark_yaml) = targets_yaml(&TargetKind::Spark, warehouse_dir);

    let yml = format!(
        "name: {name}\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  {dev_name}:\n    {dev_yaml}\n  {spark_name}:\n    {spark_yaml}\n\
         default_materialization: view\n"
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();

    for (file, content) in models {
        std::fs::write(root.join("models").join(file), content).unwrap();
    }
    root
}

/// Invokes `smelt run --project-dir <dir> --target <target_name>` and returns
/// the raw output. The caller asserts `out.status.success()` or inspects stderr.
pub fn run_smelt_on(
    project_dir: &Path,
    target_name: &str,
    extra_args: &[&str],
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_smelt"));
    cmd.args([
        "run",
        "--project-dir",
        project_dir.to_str().unwrap(),
        "--target",
        target_name,
    ])
    .args(extra_args)
    .env_remove("RUST_LOG");
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}
