#![cfg(feature = "duckdb")]
//! Phase 1 & Phase 2 regression tests from docs/plans/20260505-smelt-state-cli-bugfixes.md.
//!
//! Phase 1: build → diff must show no phantom ChangeNullability on an unchanged model.
//! Phase 2: build → delete model file → rebuild → stale schema entry is removed.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn setup_workspace(dir: &Path) {
    std::fs::create_dir_all(dir.join("models")).unwrap();
    std::fs::create_dir_all(dir.join("seeds")).unwrap();

    std::fs::write(
        dir.join("smelt.yml"),
        r#"
name: roundtrip-test
version: 1
paths:
  - models
  - seeds
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("seeds/raw_orders.csv"),
        "order_id,customer_id,amount\n1,100,29.99\n2,101,49.99\n3,100,19.99\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("models/stg_orders.sql"),
        "---\nname: stg_orders\nmaterialization: table\n---\n\
         SELECT order_id, customer_id, amount FROM smelt.raw_orders\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("models/mart_summary.sql"),
        "---\nname: mart_summary\nmaterialization: table\n---\n\
         SELECT customer_id, COUNT(*) AS order_count, SUM(amount) AS total_amount \
         FROM smelt.stg_orders GROUP BY 1\n",
    )
    .unwrap();
}

fn run_smelt(args: &[&str], dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .args(args)
        .arg("--project-dir")
        .arg(dir.to_str().unwrap())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt: {e}"))
}

/// Phase 1: `smelt build` followed by `smelt diff` must show no schema changes.
///
/// Regression for the phantom ChangeNullability bug: if type inference is not
/// deterministic between the save path (run.rs) and the diff path (diff.rs),
/// smelt diff would report spurious nullability changes.
#[test]
fn no_phantom_nullability_after_clean_build() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_workspace(dir);

    let build = run_smelt(&["build"], dir);
    assert!(
        build.status.success(),
        "smelt build failed:\nstderr: {}\nstdout: {}",
        String::from_utf8_lossy(&build.stderr),
        String::from_utf8_lossy(&build.stdout),
    );

    // Schema files must exist
    assert!(dir.join(".smelt/schemas/stg_orders.json").exists());
    assert!(dir.join(".smelt/schemas/mart_summary.json").exists());

    // smelt diff must exit 0 (no changes detected)
    let diff = run_smelt(&["diff"], dir);
    let stderr = String::from_utf8_lossy(&diff.stderr);
    let stdout = String::from_utf8_lossy(&diff.stdout);
    assert!(
        diff.status.success(),
        "smelt diff reported changes after a clean build (phantom nullability?):\nstdout: {stdout}\nstderr: {stderr}",
    );
    assert!(
        stdout.contains("No schema changes detected"),
        "expected 'No schema changes detected', got:\n{stdout}",
    );
}

/// Phase 2: after deleting a model file and rebuilding, the stale schema entry
/// must be removed from `.smelt/schemas/`.
#[test]
fn stale_schema_cleaned_after_model_deleted() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_workspace(dir);

    // Build once — both schema files appear
    let build1 = run_smelt(&["build"], dir);
    assert!(
        build1.status.success(),
        "first smelt build failed:\nstderr: {}",
        String::from_utf8_lossy(&build1.stderr),
    );
    assert!(dir.join(".smelt/schemas/mart_summary.json").exists());

    // Delete mart_summary.sql
    std::fs::remove_file(dir.join("models/mart_summary.sql")).unwrap();

    // Rebuild — mart_summary is no longer in the project
    let build2 = run_smelt(&["build"], dir);
    assert!(
        build2.status.success(),
        "second smelt build failed:\nstderr: {}",
        String::from_utf8_lossy(&build2.stderr),
    );

    // Stale schema file must be removed
    assert!(
        !dir.join(".smelt/schemas/mart_summary.json").exists(),
        "stale schema file was not cleaned up after model deletion"
    );

    // smelt diff must exit 0 — no phantom REMOVED entry
    let diff = run_smelt(&["diff"], dir);
    let stdout = String::from_utf8_lossy(&diff.stdout);
    assert!(
        diff.status.success(),
        "smelt diff reports REMOVED after stale schema cleanup:\nstdout: {stdout}",
    );
}

/// Phase 3: `smelt build --select` matching no models must emit a diagnostic to stderr.
#[test]
fn no_match_select_emits_stderr_message() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_workspace(dir);

    // First build to seed the project
    let build = run_smelt(&["build"], dir);
    assert!(build.status.success());

    // Run with a selector that matches nothing
    let out = run_smelt(&["build", "--select", "nonexistent_model_xyz"], dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "smelt build with non-matching --select should exit 0, got: {:?}",
        out.status
    );
    assert!(
        stderr.contains("no models matched"),
        "expected 'no models matched' in stderr, got: {stderr}"
    );
}
