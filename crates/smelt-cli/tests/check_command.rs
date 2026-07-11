#![cfg(feature = "duckdb")]
//! Integration tests for `smelt check` — data-quality checks against built targets.
//!
//! TDD: tests were written before the implementation to drive the feature.
//! Each test builds a real `examples/` workspace (in a temp dir) and then
//! runs `smelt check` against it, asserting the correct exit code and output.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

/// Copy a directory tree, skipping `target/` subdirectories.
fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" {
            continue;
        }
        let dest = dst.join(&name);
        if path.is_dir() {
            copy_dir(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).unwrap();
        }
    }
}

/// Set up a fresh copy of an example workspace in a temp dir and run `smelt build`.
/// Returns the `TempDir` (kept alive so the DB persists) and the workspace path.
fn build_example(example_name: &str) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let src = examples_root().join(example_name);
    let dest = tmp.path().join(example_name);
    copy_dir(&src, &dest);

    let out = Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", dest.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    // The example workspaces used here build cleanly (their committed checks
    // pass or warn — `smelt build` runs the check gate but warn/pass never set a
    // nonzero exit). Tests that need a failing check inject one into the temp
    // copy *after* this build and exercise it via the isolated `smelt check` run.
    assert!(
        out.status.success(),
        "smelt build failed for '{example_name}':\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    (tmp, dest)
}

/// Run `smelt check` against a project directory with optional extra args.
fn run_check(project_dir: &Path, extra_args: &[&str]) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("check")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .args(extra_args)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt check`: {e}"))
}

/// A `smelt.check` whose failing-rows query returns zero rows → PASS, exit 0.
#[test]
fn check_passes_on_clean_data() {
    // Build ephemeral_demo (which has daily_revenue_non_negative check that
    // queries smelt.raw_orders WHERE amount < 0; all amounts are positive so
    // the query returns zero rows → PASS).
    let (_tmp, project_dir) = build_example("ephemeral_demo");

    let out = run_check(&project_dir, &["--select", "daily_revenue_non_negative"]);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "smelt check should exit 0 on clean data.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("PASS"),
        "output should contain PASS.\nstdout: {stdout}"
    );
}

/// A check that returns rows → FAIL, exit 1; report shows violation count and sample.
#[test]
fn check_fails_on_violation() {
    // Build data_checks cleanly (its committed checks pass/warn), then inject a
    // failing error-severity check and run `smelt check` against the built data.
    // revenue.amount = 100.0, so `amount < 500` returns a row → error-severity FAIL.
    let (_tmp, project_dir) = build_example("data_checks");
    std::fs::write(
        project_dir.join("checks/must_exceed_500.sql"),
        "smelt.check must_exceed_500 AS (\n    SELECT order_id, amount FROM smelt.revenue WHERE amount < 500\n)\n",
    )
    .unwrap();

    let out = run_check(&project_dir, &["--select", "must_exceed_500"]);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "smelt check should exit 1 on violation.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("FAIL"),
        "output should contain FAIL.\nstdout: {stdout}"
    );
    // Should report violation count (1 row)
    assert!(
        stdout.contains('1'),
        "output should show violation count.\nstdout: {stdout}"
    );
}

/// A `warn`-severity check with violations → WARN in output, but exit 0.
#[test]
fn warn_severity_does_not_fail() {
    // Build data_checks (amount_above_500_warn has severity: warn).
    let (_tmp, project_dir) = build_example("data_checks");

    let out = run_check(&project_dir, &["--select", "amount_above_500_warn"]);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "smelt check should exit 0 when only warn-severity checks fail.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("WARN"),
        "output should contain WARN.\nstdout: {stdout}"
    );
    // FAIL must NOT appear (no error-severity violations)
    assert!(
        !stdout.contains("FAIL"),
        "output must not contain FAIL for warn-only run.\nstdout: {stdout}"
    );
}

/// Running `smelt check` before building the referenced model → CheckTargetNotBuilt, exit 1.
/// Never a silent pass.
#[test]
fn check_on_unbuilt_model_is_loud() {
    // Set up data_checks but do NOT run `smelt build`.
    let tmp = TempDir::new().unwrap();
    let src = examples_root().join("data_checks");
    let dest = tmp.path().join("data_checks");
    copy_dir(&src, &dest);

    let out = run_check(&dest, &[]);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "smelt check should exit 1 when referenced model is not built.\nstdout: {stdout}\nstderr: {stderr}"
    );
    // Must report CheckTargetNotBuilt or equivalent loud error — not a silent pass.
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.to_lowercase().contains("not built")
            || combined.to_lowercase().contains("notbuilt")
            || combined.to_lowercase().contains("does not exist")
            || combined.to_lowercase().contains("fail"),
        "output should mention the unbuilt target.\ncombined: {combined}"
    );
}

/// `--select` narrows by check name via substring, consistent with `smelt test --select`.
#[test]
fn check_select_substring() {
    // Build data_checks then run only the passing check (no_negative_amounts).
    let (_tmp, project_dir) = build_example("data_checks");

    // --select "no_negative" only matches `no_negative_amounts` (PASSes).
    // `amount_above_500_warn` (the other committed check) is excluded.
    let out = run_check(&project_dir, &["--select", "no_negative"]);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "smelt check --select 'no_negative' should exit 0 (only passing check selected).\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("PASS"),
        "output should contain PASS.\nstdout: {stdout}"
    );
    // The non-matching check must not appear in output
    assert!(
        !stdout.contains("amount_above_500_warn"),
        "non-matching check must be excluded by --select.\nstdout: {stdout}"
    );
}
