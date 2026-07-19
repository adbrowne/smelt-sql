#![cfg(feature = "duckdb")]
//! Integration tests for `columns.<c>.tests` — declarative column tests
//! (`docs/specs/data_tests.md`).
//!
//! TDD: tests were written before the implementation to drive the feature.
//! Each test copies a real `examples/` workspace into a temp dir, injects
//! frontmatter declaring `columns.<c>.tests`, and runs the real `smelt`
//! binary against it — the same harness style as `tests/check_command.rs`.

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

/// Copy `examples/data_checks` into a fresh temp dir, without building it.
fn copy_data_checks() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let src = examples_root().join("data_checks");
    let dest = tmp.path().join("data_checks");
    copy_dir(&src, &dest);
    (tmp, dest)
}

fn run_smelt(project_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(smelt_bin())
        .args(args)
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt {}`: {e}", args.join(" ")))
}

/// A `columns.<c>.tests` entry naming a column absent from the model's
/// inferred output schema is a hard diagnostic — contrast with the
/// silent-drop rule for other `columns:` keys
/// (`docs/specs/data_tests.md` §"Fail-loud validation").
#[test]
fn test_on_unknown_column_is_diagnostic() {
    let (_tmp, project_dir) = copy_data_checks();

    // `revenue.sql` originally has no frontmatter and outputs `order_id`,
    // `amount`. Declare a test on a column that does not exist.
    std::fs::write(
        project_dir.join("models/revenue.sql"),
        r#"---
name: revenue
columns:
  order_id:
    tests:
      - not_null
  bogus_column:
    tests:
      - not_null
---
SELECT
    1 AS order_id,
    100.0 AS amount
"#,
    )
    .unwrap();

    let out = run_smelt(&project_dir, &["run", "--dry-run"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !out.status.success(),
        "smelt run --dry-run should fail loudly on a test naming an unmodeled column.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        combined.contains("bogus_column"),
        "diagnostic should name the offending column.\ncombined: {combined}"
    );
}

/// A `not_null` test on a column the type checker already proves
/// non-nullable reports `proven` — no scan is emitted for it
/// (`docs/specs/data_tests.md` §Semantics "Resolution order").
#[test]
fn proven_not_null_emits_no_scan() {
    let (_tmp, project_dir) = copy_data_checks();

    // Both `order_id` and `amount` are literal constants (Computed,
    // non-nullable) — the type checker proves `order_id IS NOT NULL`.
    std::fs::write(
        project_dir.join("models/revenue.sql"),
        r#"---
name: revenue
columns:
  order_id:
    tests:
      - not_null
---
SELECT
    1 AS order_id,
    100.0 AS amount
"#,
    )
    .unwrap();

    // The declarative-tests proof pass needs no built target — run `smelt
    // check` directly (no `smelt build` first).
    let out = run_smelt(&project_dir, &["check"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stdout.contains("PROVEN") && stdout.contains("revenue.order_id.not_null"),
        "output should report the not_null test as proven, with no scan.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// A `unique` test on the model's own declared `unique_key:` reports
/// `proven` — the grain key is the proof, no scan needed
/// (`docs/specs/data_tests.md` §Semantics "Resolution order").
#[test]
fn proven_unique_via_grain_key() {
    let (_tmp, project_dir) = copy_data_checks();

    std::fs::write(
        project_dir.join("models/revenue.sql"),
        r#"---
name: revenue
materialization: table
refresh: incremental
unique_key: [order_id]
columns:
  order_id:
    tests:
      - unique
---
SELECT
    1 AS order_id,
    100.0 AS amount
"#,
    )
    .unwrap();

    let out = run_smelt(&project_dir, &["check"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stdout.contains("PROVEN") && stdout.contains("revenue.order_id.unique"),
        "output should report the unique test as proven via the declared unique_key.\nstdout: {stdout}\nstderr: {stderr}"
    );
}
