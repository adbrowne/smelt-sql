#![cfg(feature = "duckdb")]
//! `smelt list` surfaces external steps as a distinct, selectable kind
//! (phase 3 of `docs/outcomes/20260906-external-dag-steps`).
//!
//! Spec: `docs/specs/cli.md` §"`smelt list`"; `docs/specs/model_selection.md`
//! §"Selection methods".

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

/// Scaffold a minimal project via `smelt init`, then add a per-entity source
/// YAML, an external step that produces it, and a model that consumes it.
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

    std::fs::create_dir_all(project_dir.join("models").join("sources")).unwrap();
    std::fs::write(
        project_dir
            .join("models")
            .join("sources")
            .join("raw_events.yml"),
        "columns:\n  - name: id\n    type: INTEGER\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("loader.yml"),
        "external_step:\n  produces:\n    - smelt.sources.raw_events\n  command: [\"bash\", \"loader.sh\"]\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("consumer.sql"),
        "SELECT * FROM smelt.sources.raw_events\n",
    )
    .unwrap();

    project_dir
}

#[test]
fn list_shows_external_step_kind() {
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

    let loader = entries
        .iter()
        .find(|e| e["address"] == "smelt.loader")
        .unwrap_or_else(|| panic!("expected smelt.loader in JSON output:\n{stdout}"));
    assert_eq!(loader["kind"], "external_step");
    assert_eq!(
        loader["produces"],
        serde_json::json!(["smelt.sources.raw_events"])
    );
}

#[test]
fn list_select_downstream_model_includes_producing_step() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold(&tmp);

    // A bare step selector selects only the step — no models.
    let out = run(&project_dir, &["list", "--select", "loader"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}");
    assert!(stdout.contains("smelt.loader"), "{stdout}");
    assert!(
        !stdout.contains("smelt.consumer") && !stdout.contains("smelt.orders_summary"),
        "a bare step selector must not pull in any model:\n{stdout}"
    );

    // Upstream traversal from the consumer reaches the producing step.
    let out = run(&project_dir, &["list", "--select", "+consumer"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}");
    assert!(
        stdout.contains("smelt.loader") && stdout.contains("external_step"),
        "expected +consumer to reach the producing step:\n{stdout}"
    );
    assert!(stdout.contains("smelt.consumer"), "{stdout}");
}
