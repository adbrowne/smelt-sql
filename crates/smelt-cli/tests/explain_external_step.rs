#![cfg(feature = "duckdb")]
//! `smelt explain` renders external steps (phase 6 of
//! `docs/outcomes/20260906-external-dag-steps`).
//!
//! Spec: `docs/specs/cli.md` §"`smelt explain --json` output schema",
//! §"`smelt explain <external step>`".

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
/// The step's `command:` touches a marker file so tests can assert `explain`
/// never spawns it.
fn scaffold(tmp: &TempDir) -> (PathBuf, PathBuf) {
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

    let marker = tmp.path().join("step_ran.marker");

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
        format!(
            "external_step:\n  description: \"loads raw events\"\n  produces:\n    - smelt.sources.raw_events\n  command: [\"touch\", \"{}\"]\n  cadence: '1 day'\n",
            marker.to_str().unwrap().replace('\\', "\\\\")
        ),
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("consumer.sql"),
        "SELECT * FROM smelt.sources.raw_events\n",
    )
    .unwrap();

    (project_dir, marker)
}

fn scaffold_without_steps(tmp: &TempDir) -> PathBuf {
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

#[test]
fn whole_project_json_carries_steps() {
    let tmp = TempDir::new().unwrap();
    let (project_dir, marker) = scaffold(&tmp);

    let out = run(&project_dir, &["explain", "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    let step = &parsed["external_steps"]["loader"];
    assert_eq!(
        step["produces"],
        serde_json::json!(["smelt.sources.raw_events"])
    );
    assert_eq!(
        step["command"],
        serde_json::json!(["touch", marker.to_str().unwrap()])
    );
    assert_eq!(step["consumers"], serde_json::json!(["consumer"]));
}

#[test]
fn whole_project_json_omits_key_without_steps() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold_without_steps(&tmp);

    let out = run(&project_dir, &["explain", "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}");

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    assert!(
        parsed.get("external_steps").is_none(),
        "expected no external_steps key when the project declares no step:\n{stdout}"
    );
}

#[test]
fn execution_order_and_models_stay_model_only() {
    let tmp = TempDir::new().unwrap();
    let (project_dir, _marker) = scaffold(&tmp);

    let out = run(&project_dir, &["explain", "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}");

    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let execution_order = parsed["execution_order"]
        .as_array()
        .expect("execution_order array");
    assert!(
        execution_order.iter().all(|v| v.as_str() != Some("loader")),
        "step must not appear in execution_order:\n{stdout}"
    );
    let models = parsed["models"].as_object().expect("models object");
    assert!(
        !models.contains_key("loader"),
        "step must not appear in models:\n{stdout}"
    );
}

#[test]
fn whole_project_text_lists_steps() {
    let tmp = TempDir::new().unwrap();
    let (project_dir, _marker) = scaffold(&tmp);

    let out = run(&project_dir, &["explain"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}");
    assert!(stdout.contains("External steps:"), "{stdout}");
    assert!(stdout.contains("loader"), "{stdout}");
    assert!(stdout.contains("smelt.sources.raw_events"), "{stdout}");
}

#[test]
fn select_narrows_steps() {
    let tmp = TempDir::new().unwrap();
    let (project_dir, _marker) = scaffold(&tmp);

    let out = run(
        &project_dir,
        &["explain", "--json", "--select", "+consumer"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        parsed["external_steps"].get("loader").is_some(),
        "--select +consumer should keep the producing step:\n{stdout}"
    );

    let out = run(
        &project_dir,
        &["explain", "--json", "--select", "orders_summary"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        parsed.get("external_steps").is_none()
            || parsed["external_steps"].as_object().unwrap().is_empty(),
        "--select orders_summary (unrelated) should drop the step:\n{stdout}"
    );
}

#[test]
fn explain_step_text_renders_contract() {
    let tmp = TempDir::new().unwrap();
    let (project_dir, _marker) = scaffold(&tmp);

    let out = run(&project_dir, &["explain", "loader"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("smelt.sources.raw_events"), "{stdout}");
    assert!(stdout.contains("touch"), "{stdout}");
    assert!(stdout.contains("consumer"), "{stdout}");
    assert!(
        stdout.contains("does not author or parse"),
        "expected the not-authored sentence:\n{stdout}"
    );
    assert!(
        !stdout.contains("Refusals") && !stdout.contains("maintenance plan"),
        "a step's report must not look like a maintenance plan:\n{stdout}"
    );
}

#[test]
fn explain_step_json_shape() {
    let tmp = TempDir::new().unwrap();
    let (project_dir, _marker) = scaffold(&tmp);

    let out = run(&project_dir, &["explain", "loader", "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}");

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    assert_eq!(parsed["kind"], "external_step");
    assert_eq!(parsed["address"], "smelt.loader");
    assert_eq!(
        parsed["produces"],
        serde_json::json!(["smelt.sources.raw_events"])
    );
    assert_eq!(parsed["consumers"], serde_json::json!(["consumer"]));
    assert_eq!(parsed["cadence"], "1 day");
}

#[test]
fn explain_step_never_spawns_the_command() {
    let tmp = TempDir::new().unwrap();
    let (project_dir, marker) = scaffold(&tmp);

    let out = run(&project_dir, &["explain", "loader"]);
    assert!(out.status.success());
    assert!(
        !marker.exists(),
        "smelt explain loader must never spawn the command"
    );

    let out = run(&project_dir, &["explain", "loader", "--json"]);
    assert!(out.status.success());
    assert!(
        !marker.exists(),
        "smelt explain loader --json must never spawn the command"
    );
}

#[test]
fn explain_step_rejects_plan_flags() {
    let tmp = TempDir::new().unwrap();
    let (project_dir, _marker) = scaffold(&tmp);

    let out = run(&project_dir, &["explain", "loader", "--show-sql"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("loader"), "{stderr}");

    let out = run(
        &project_dir,
        &["explain", "loader", "--period", "2026-01-01..2026-01-02"],
    );
    assert_eq!(out.status.code(), Some(2));

    let out = run(
        &project_dir,
        &[
            "explain",
            "loader",
            "--show-sql",
            "--technique",
            "delete_insert",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn explain_unknown_target_still_not_found() {
    let tmp = TempDir::new().unwrap();
    let (project_dir, _marker) = scaffold(&tmp);

    let out = run(&project_dir, &["explain", "totally_unknown_thing"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("Not found") || stderr.contains("No"),
        "expected the existing not-found error, got:\n{stderr}"
    );
}
