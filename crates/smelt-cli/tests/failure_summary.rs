#![cfg(feature = "duckdb")]
//! TDD tests for the grouped failure-summary UX
//! (`docs/plans/20260719-prod-w3-adoption.md` Phase 6: "Failure-summary
//! UX"; `docs/specs/cli.md` §Semantics "Failure summary").
//!
//! `crates/smelt-cli/tests/run_report.rs::failure_summary_lists_all_failed_models`
//! (W2 Phase 8) already covers `smelt run` grouping every failed model's
//! first error line into one block. This file adds: per-failure hints,
//! `smelt build` parity (Phase 8 only wired `run.rs`), and the
//! success-run-prints-nothing case.

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn scaffold(tmp: &TempDir) -> PathBuf {
    let project_dir = tmp.path().join("proj");
    let init_out = Command::new(smelt_bin())
        .arg("init")
        .arg(&project_dir)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt init`: {e}"));
    assert!(
        init_out.status.success(),
        "smelt init should succeed.\nstderr: {}",
        String::from_utf8_lossy(&init_out.stderr)
    );
    project_dir
}

/// Two independently-failing models each get a one-line cause AND a hint in
/// the grouped summary — not just the cause line.
#[test]
fn multi_model_failure_grouped_summary() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold(&tmp);

    // `materialization: table` forces an eager `CREATE TABLE AS`, so the
    // cast error surfaces at execution time (matches
    // `run_report.rs::failure_summary_lists_all_failed_models`'s fixture
    // shape — the default `view` materialization only errors lazily).
    std::fs::write(
        project_dir.join("models").join("bad_a.sql"),
        "---\nmaterialization: table\n---\nSELECT CAST('not_a_number' AS INT) AS id\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("bad_b.sql"),
        "---\nmaterialization: table\n---\nSELECT CAST('also_not_a_number' AS INT) AS id\n",
    )
    .unwrap();

    let out = Command::new(smelt_bin())
        .arg("run")
        .arg("--select")
        .arg("bad_a")
        .arg("--select")
        .arg("bad_b")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"));

    assert!(
        !out.status.success(),
        "run should fail — both bad_a and bad_b error at execution time"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bad_a") && stderr.contains("bad_b"),
        "failure summary must name both failed models:\n{stderr}"
    );
    // Each failed model's block carries its own hint line, not just its
    // cause line.
    let hint_lines = stderr.lines().filter(|l| l.contains("hint:")).count();
    assert_eq!(
        hint_lines, 2,
        "expected one hint line per failed model:\n{stderr}"
    );
}

/// A green run prints no failure block at all.
#[test]
fn success_run_prints_no_failure_block() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold(&tmp);

    // `smelt run` alone doesn't seed `raw_orders` — use `build`, which runs
    // the seed lifecycle first, so the scaffolded project actually succeeds.
    let out = Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    assert!(
        out.status.success(),
        "scaffolded project should build cleanly.\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("failed") && !stderr.contains("hint:"),
        "a successful run must not print a failure summary block:\n{stderr}"
    );
}

/// `smelt build` gets the same grouped failure summary as `smelt run` —
/// Phase 8 (W2) only wired the CLI-side call into `run.rs`.
#[test]
fn build_prints_failure_summary_too() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold(&tmp);

    std::fs::write(
        project_dir.join("models").join("bad_a.sql"),
        "---\nmaterialization: table\n---\nSELECT CAST('not_a_number' AS INT) AS id\n",
    )
    .unwrap();

    let out = Command::new(smelt_bin())
        .arg("build")
        .arg("--select")
        .arg("bad_a")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    assert!(
        !out.status.success(),
        "build should fail — bad_a errors at execution time"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bad_a") && stderr.contains("hint:"),
        "smelt build must print the same grouped failure summary as smelt run:\n{stderr}"
    );
}
