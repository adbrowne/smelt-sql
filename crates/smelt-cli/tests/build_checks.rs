#![cfg(feature = "duckdb")]
//! Integration tests for smelt build integrating smelt.check data-quality checks.
//!
//! TDD: tests were written before the implementation to drive Phase 4.
//! The build lifecycle runs checks after each model materializes:
//!   - error-severity violation: skip every downstream model, build exits 1.
//!   - warn-severity violation: report and continue, build exits 0.
//!   - passing check: build continues normally.

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

/// Set up a fresh copy of the `build_checks_skip` fixture in a temp dir and
/// optionally modify `models/a.sql` to return rows (making the error check fail).
fn stage_fixture(make_a_nonempty: bool) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let src = examples_root().join("build_checks_skip");
    let dest = tmp.path().join("build_checks_skip");
    copy_dir(&src, &dest);

    if make_a_nonempty {
        // Override a.sql so it returns a row — this triggers the check.
        std::fs::write(dest.join("models/a.sql"), "SELECT 1 AS id\n").unwrap();
    }

    (tmp, dest)
}

/// Same as stage_fixture but swap out the error-severity check for a warn check only.
fn stage_fixture_warn_only(make_a_nonempty: bool) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let src = examples_root().join("build_checks_skip");
    let dest = tmp.path().join("build_checks_skip");
    copy_dir(&src, &dest);

    if make_a_nonempty {
        std::fs::write(dest.join("models/a.sql"), "SELECT 1 AS id\n").unwrap();
    }

    // Remove the error-severity check; keep only the warn check.
    std::fs::remove_file(dest.join("checks/a_must_be_empty.sql")).unwrap();

    (tmp, dest)
}

fn run_build(project_dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"))
}

fn check_table_exists(project_dir: &Path, table_name: &str) -> bool {
    let db_path = project_dir.join("target/dev.duckdb");
    if !db_path.exists() {
        return false;
    }
    let conn = match duckdb::Connection::open(&db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let query = format!(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'main' AND table_name = '{table_name}'"
    );
    let mut stmt = conn.prepare(&query).unwrap();
    let count: i64 = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .next()
        .and_then(|r| r.ok())
        .unwrap_or(0);
    count > 0
}

/// Error-severity check that fails → A materializes, check fails, B is skipped,
/// build exits nonzero, B's relation is absent.
#[test]
fn build_error_check_skips_downstream() {
    // Make a.sql return a row — the check "SELECT id FROM smelt.a" returns rows → FAIL.
    let (_tmp, project_dir) = stage_fixture(true);

    let out = run_build(&project_dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Build must exit 1.
    assert!(
        !out.status.success(),
        "smelt build must exit 1 when an error-severity check fails.\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Model A must have been built (check runs against it).
    assert!(
        check_table_exists(&project_dir, "a"),
        "model A must have materialized before the check ran.\nstdout: {stdout}"
    );

    // Model B must be absent — it was skipped due to the check failure on A.
    assert!(
        !check_table_exists(&project_dir, "b"),
        "model B must be skipped (absent from the target) when A's check fails.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// Warn-severity check that fails → B still builds, build exits 0, WARN reported.
#[test]
fn build_warn_check_does_not_skip() {
    // A returns a row, but the only check is warn-severity.
    let (_tmp, project_dir) = stage_fixture_warn_only(true);

    let out = run_build(&project_dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Build must exit 0.
    assert!(
        out.status.success(),
        "smelt build must exit 0 when only warn-severity checks fail.\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Both A and B must be present.
    assert!(
        check_table_exists(&project_dir, "a"),
        "model A must have materialized.\nstdout: {stdout}"
    );
    assert!(
        check_table_exists(&project_dir, "b"),
        "model B must still build when only warn checks fail.\nstdout: {stdout}\nstderr: {stderr}"
    );

    // WARN must appear in output.
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.to_lowercase().contains("warn"),
        "output should mention WARN.\ncombined: {combined}"
    );
}

/// Passing check → build identical to no check (both A and B present, exit 0).
#[test]
fn build_passing_check_is_transparent() {
    // Default fixture: a.sql returns no rows → check returns 0 rows → PASS.
    let (_tmp, project_dir) = stage_fixture(false);

    let out = run_build(&project_dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "smelt build must exit 0 when all checks pass.\nstdout: {stdout}\nstderr: {stderr}"
    );

    assert!(
        check_table_exists(&project_dir, "a"),
        "model A must have materialized.\nstdout: {stdout}"
    );
    assert!(
        check_table_exists(&project_dir, "b"),
        "model B must have materialized after a passing check.\nstdout: {stdout}\nstderr: {stderr}"
    );
}
