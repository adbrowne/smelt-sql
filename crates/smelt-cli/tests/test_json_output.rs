#![cfg(feature = "duckdb")]
//! Integration tests for `smelt test --json` output format.
//! Verifies the machine-readable JSON output used by the VSCode test runner.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn smelt_yml(name: &str) -> String {
    format!(
        "name: {name}\nversion: 1\npaths:\n  - models\n  - tests\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n"
    )
}

fn init_workspace(root: &std::path::Path, name: &str) {
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml(name)).unwrap();
    std::fs::write(root.join("models/simple.sql"), "SELECT 1 AS x\n").unwrap();
}

/// Workspace with both a passing and a failing test (for no-select coverage).
fn stage_test_workspace(tmp: &TempDir, name: &str) -> PathBuf {
    let root = tmp.path().join(name);
    init_workspace(&root, name);
    std::fs::write(
        root.join("tests/test_pass.sql"),
        "--- name: test_pass ---\nmaterialization: test\ntest:\n  model: simple\n  expect:\n    - {x: 1}\n---\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tests/test_fail.sql"),
        "--- name: test_fail ---\nmaterialization: test\ntest:\n  model: simple\n  expect:\n    - {x: 2}\n---\n",
    )
    .unwrap();
    root
}

/// Workspace with only the passing test — used to isolate exactly one result.
fn stage_passing_workspace(tmp: &TempDir, name: &str) -> PathBuf {
    let root = tmp.path().join(name);
    init_workspace(&root, name);
    std::fs::write(
        root.join("tests/test_pass.sql"),
        "--- name: test_pass ---\nmaterialization: test\ntest:\n  model: simple\n  expect:\n    - {x: 1}\n---\n",
    )
    .unwrap();
    root
}

/// Workspace with only the failing test — used to isolate exactly one result.
fn stage_failing_workspace(tmp: &TempDir, name: &str) -> PathBuf {
    let root = tmp.path().join(name);
    init_workspace(&root, name);
    std::fs::write(
        root.join("tests/test_fail.sql"),
        "--- name: test_fail ---\nmaterialization: test\ntest:\n  model: simple\n  expect:\n    - {x: 2}\n---\n",
    )
    .unwrap();
    root
}

fn run_smelt_test(project_dir: &Path, extra_args: &[&str]) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("test")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .args(extra_args)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt test`: {e}"))
}

/// `smelt test --json` must output valid JSON with a `results` array.
#[test]
fn test_json_flag_outputs_valid_json() {
    let tmp = TempDir::new().unwrap();
    let workspace = stage_test_workspace(&tmp, "json_valid_ws");

    let output = run_smelt_test(&workspace, &["--json"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout must be valid JSON with --json flag\nError: {e}\nStdout:\n{stdout}\nStderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });

    assert!(
        parsed.get("results").is_some(),
        "JSON output must have a 'results' key; got: {parsed}"
    );
}

/// `smelt test --json` must mark the passing test as `status: "pass"`.
#[test]
fn test_json_pass_status() {
    let tmp = TempDir::new().unwrap();
    let workspace = stage_passing_workspace(&tmp, "json_pass_ws");

    let output = run_smelt_test(&workspace, &["--json"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout not valid JSON: {e}\nStdout:\n{stdout}"));

    let results = parsed["results"].as_array().expect("results must be array");
    assert_eq!(
        results.len(),
        1,
        "should have exactly 1 result; got: {parsed}"
    );

    let status = results[0]["status"].as_str().unwrap_or("");
    assert_eq!(
        status, "pass",
        "test_pass must have status 'pass'; got: {parsed}"
    );

    assert!(
        results[0].get("duration_ms").is_some(),
        "result must include duration_ms; got: {parsed}"
    );
}

/// `smelt test --json` must mark the failing test as `status: "fail"` with a message.
#[test]
fn test_json_fail_status() {
    let tmp = TempDir::new().unwrap();
    let workspace = stage_failing_workspace(&tmp, "json_fail_ws");

    let output = run_smelt_test(&workspace, &["--json"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout not valid JSON: {e}\nStdout:\n{stdout}"));

    let results = parsed["results"].as_array().expect("results must be array");
    assert_eq!(
        results.len(),
        1,
        "should have exactly 1 result; got: {parsed}"
    );

    let status = results[0]["status"].as_str().unwrap_or("");
    assert_eq!(
        status, "fail",
        "test_fail must have status 'fail'; got: {parsed}"
    );

    let message = results[0]["message"].as_str().unwrap_or("");
    assert!(
        !message.is_empty(),
        "failing test must have a non-empty message; got: {parsed}"
    );
}

/// `smelt test --json` must exit 0 even when tests fail (callers check per-test status).
#[test]
fn test_json_exits_zero_on_failure() {
    let tmp = TempDir::new().unwrap();
    let workspace = stage_failing_workspace(&tmp, "json_exit_ws");

    let output = run_smelt_test(&workspace, &["--json"]);

    assert!(
        output.status.success(),
        "smelt test --json must exit 0 even when tests fail; got {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
