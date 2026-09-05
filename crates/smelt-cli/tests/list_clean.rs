#![cfg(feature = "duckdb")]
//! Integration tests for `smelt list` and `smelt clean`.
//!
//! TDD: written before the implementation to drive the feature, following
//! the same real-binary-subprocess harness as `tests/init_command.rs`.
//!
//! Spec: `docs/specs/cli.md` §"`smelt list`" / §"`smelt clean`".

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn run(project_dir: &Path, args: &[&str]) -> Output {
    Command::new(smelt_bin())
        .args(args)
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt {args:?}`: {e}"))
}

/// Scaffold a minimal project via `smelt init`, then add a second model with
/// an explicit `materialization: table` override so `smelt list` has more
/// than one materialization value to distinguish.
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
    // `smelt init`'s scaffold declares no `state:` key, so it defaults to
    // `state.mode: stateless` (`docs/specs/state.md` §"`state.mode` and
    // what each posture provides"); `clean_removes_artifacts_preserves_state`
    // needs a posture that actually writes `.smelt/`.
    let smelt_yml_path = project_dir.join("smelt.yml");
    let mut smelt_yml = std::fs::read_to_string(&smelt_yml_path).unwrap();
    smelt_yml.push_str("state:\n  mode: intervals\n");
    std::fs::write(&smelt_yml_path, smelt_yml).unwrap();

    std::fs::write(
        project_dir.join("models").join("orders_table.sql"),
        "---\nmaterialization: table\n---\nSELECT * FROM smelt.orders_summary\n",
    )
    .unwrap();

    project_dir
}

#[test]
fn list_shows_all_models_with_kinds() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold(&tmp);

    let out = run(&project_dir, &["list"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "smelt list should exit 0.\nstdout: {stdout}\nstderr: {stderr}"
    );

    assert!(
        stdout.contains("smelt.orders_summary") && stdout.contains("model"),
        "expected the scaffolded view model in the listing:\n{stdout}"
    );
    assert!(
        stdout.contains("smelt.orders_table") && stdout.contains("table"),
        "expected the table-materialized model in the listing:\n{stdout}"
    );
    assert!(
        stdout.contains("smelt.raw_orders") && stdout.contains("seed"),
        "expected the scaffolded seed in the listing:\n{stdout}"
    );

    // --select narrows the (model) result set.
    let out = run(&project_dir, &["list", "--select", "orders_table"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(stdout.contains("smelt.orders_table"), "{stdout}");
    assert!(
        !stdout.contains("smelt.orders_summary"),
        "orders_summary should be filtered out by --select:\n{stdout}"
    );
}

#[test]
fn list_json_output() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold(&tmp);

    let out = run(&project_dir, &["list", "--format", "json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "smelt list --format json should exit 0.\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    let entries = parsed.as_array().expect("expected a JSON array");
    assert!(!entries.is_empty(), "expected at least one entry");

    let orders_table = entries
        .iter()
        .find(|e| e["address"] == "smelt.orders_table")
        .unwrap_or_else(|| panic!("expected smelt.orders_table in JSON output:\n{stdout}"));
    assert_eq!(orders_table["kind"], "model");
    assert_eq!(orders_table["materialization"], "table");

    let seed = entries
        .iter()
        .find(|e| e["address"] == "smelt.raw_orders")
        .unwrap_or_else(|| panic!("expected smelt.raw_orders in JSON output:\n{stdout}"));
    assert_eq!(seed["kind"], "seed");
}

#[test]
fn clean_removes_artifacts_preserves_state() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold(&tmp);

    let build_out = run(&project_dir, &["build"]);
    assert!(
        build_out.status.success(),
        "smelt build should succeed.\nstderr: {}",
        String::from_utf8_lossy(&build_out.stderr)
    );
    assert!(
        project_dir.join(".smelt").is_dir(),
        "expected .smelt/ state directory after build"
    );

    // A build artifact under target/ (the directory `smelt docs generate`
    // and other artifact-producing commands write to) — created directly
    // here rather than via `smelt docs generate` to keep this test focused
    // on `smelt clean`'s own behavior.
    std::fs::create_dir_all(project_dir.join("target").join("docs")).unwrap();
    std::fs::write(
        project_dir.join("target").join("docs").join("index.md"),
        "# catalog\n",
    )
    .unwrap();

    let clean_out = run(&project_dir, &["clean"]);
    let stdout = String::from_utf8_lossy(&clean_out.stdout);
    assert!(
        clean_out.status.success(),
        "smelt clean should exit 0.\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&clean_out.stderr)
    );
    assert!(
        !project_dir.join("target").exists(),
        "target/ should be removed by smelt clean"
    );
    assert!(
        project_dir.join(".smelt").is_dir(),
        "smelt clean must never remove .smelt/ state"
    );
    assert!(
        project_dir.join("dev.duckdb").exists(),
        "smelt clean must never touch the target database file"
    );
    assert!(
        stdout.contains("docs"),
        "smelt clean should print what it deleted:\n{stdout}"
    );
}

#[test]
fn clean_noop_when_no_target_dir() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold(&tmp);

    let out = run(&project_dir, &["clean"]);
    assert!(
        out.status.success(),
        "smelt clean should exit 0 even when target/ never existed.\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
