#![cfg(feature = "duckdb")]
//! Exit-code contract tests (`docs/specs/cli.md` §"Exit codes"):
//! `1` = detected failure, `2` = usage/config error, `0` = success.
//!
//! TDD: written before the classification layer (Phase 4 of
//! `docs/plans/20260719-prod-w1-fail-loud.md`) — before this phase these all
//! exited `1` uniformly via `anyhow`/`std::process::exit(1)`.

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

/// A malformed `smelt.yml` (unparsable YAML) → usage/config error, exit `2`.
#[test]
fn config_error_exits_two() {
    let tmp = TempDir::new().unwrap();
    let project_dir = tmp.path().join("broken_config");
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    // Invalid YAML: unterminated flow mapping.
    std::fs::write(project_dir.join("smelt.yml"), "name: [unterminated\n").unwrap();

    let out = Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "malformed smelt.yml should exit 2.\nstdout: {}\nstderr: {stderr}",
        String::from_utf8_lossy(&out.stdout),
    );
    assert!(
        !stderr.is_empty(),
        "expected an error message on stderr, got none"
    );
}

/// No `smelt.yml`/`models/` anywhere up the tree → project root not found →
/// usage/config error, exit `2`.
#[test]
fn missing_workspace_exits_two() {
    let tmp = TempDir::new().unwrap();
    // An empty directory with no smelt.yml/smelt.yaml/models/ ancestor. Use a
    // fresh subdirectory under the tempdir so a real project root a few
    // levels up (e.g. this repo's own smelt.yml, if any exists) can't be
    // found by the 5-level walk-up.
    let empty_dir = tmp.path().join("nowhere");
    std::fs::create_dir_all(&empty_dir).unwrap();

    let out = Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", empty_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "missing project root should exit 2.\nstdout: {}\nstderr: {stderr}",
        String::from_utf8_lossy(&out.stdout),
    );
    assert!(
        !stderr.is_empty(),
        "expected an error message on stderr, got none"
    );
}

/// An `error`-severity check with violations → detected failure, exit `1`.
#[test]
fn failed_check_exits_one() {
    let tmp = TempDir::new().unwrap();
    let src = examples_root().join("data_checks");
    let dest = tmp.path().join("data_checks");
    copy_dir(&src, &dest);

    let build_out = Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", dest.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));
    assert!(
        build_out.status.success(),
        "smelt build should succeed for data_checks.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr),
    );

    // revenue.amount = 100.0, so `amount < 500` returns a row → error-severity FAIL.
    std::fs::write(
        dest.join("checks/must_exceed_500.sql"),
        "smelt.check must_exceed_500 AS (\n    SELECT order_id, amount FROM smelt.revenue WHERE amount < 500\n)\n",
    )
    .unwrap();

    let out = Command::new(smelt_bin())
        .arg("check")
        .args(["--project-dir", dest.to_str().unwrap()])
        .args(["--select", "must_exceed_500"])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt check`: {e}"));

    assert_eq!(
        out.status.code(),
        Some(1),
        "error-severity check violation should exit 1.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// A `warn`-severity check with violations → exit `0` (spec rule: warn never
/// affects the exit code).
#[test]
fn warn_severity_check_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let src = examples_root().join("data_checks");
    let dest = tmp.path().join("data_checks");
    copy_dir(&src, &dest);

    let build_out = Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", dest.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));
    assert!(
        build_out.status.success(),
        "smelt build should succeed for data_checks.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr),
    );

    let out = Command::new(smelt_bin())
        .arg("check")
        .args(["--project-dir", dest.to_str().unwrap()])
        .args(["--select", "amount_above_500_warn"])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt check`: {e}"));

    assert_eq!(
        out.status.code(),
        Some(0),
        "warn-severity-only violations must exit 0.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
