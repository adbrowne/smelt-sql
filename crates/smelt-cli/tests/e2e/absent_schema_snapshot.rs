//! End-to-end coverage for `smelt diff`'s absent-schema-snapshot degradation
//! (`docs/outcomes/20260816-state-residency/phases/03-plan.md`,
//! `docs/specs/schema_evolution.md`): a deployed-schema snapshot missing
//! because the project's `state.mode` excludes it (`stateless`) reports
//! `new` plus a say-so line, distinct from a snapshot missing because the
//! model really is new or its file was deleted under a snapshot-writing
//! posture — that case stays a plain `new`, no refusal.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    )
}

fn stage_workspace(tmp: &TempDir, state_mode: &str) -> PathBuf {
    let root = tmp.path().join("absent_snapshot");
    write_file(
        &root.join("smelt.yml"),
        &format!(
            r#"name: absent_snapshot
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
default_materialization: table
state:
  mode: {state_mode}
"#
        ),
    );
    write_file(
        &root.join("models/base.sql"),
        "SELECT 1 AS id, 'hello' AS label\n",
    );
    root
}

fn run_build(project_dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .args(["build", "--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"))
}

fn run_diff(project_dir: &Path, json: bool) -> std::process::Output {
    let mut args = vec![
        "diff".to_string(),
        "--project-dir".to_string(),
        project_dir.to_str().unwrap().to_string(),
    ];
    if json {
        args.push("--json".to_string());
    }
    Command::new(smelt_bin())
        .args(&args)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt diff`: {e}"))
}

#[test]
fn diff_reports_new_and_says_state_excluded_under_stateless() {
    let tmp = TempDir::new().expect("tempdir");
    let root = stage_workspace(&tmp, "stateless");

    let build = run_build(&root);
    assert!(
        build.status.success(),
        "build must succeed: {}",
        combined_output(&build)
    );
    assert!(
        !root.join(".smelt").exists(),
        "a stateless project must never create .smelt/"
    );

    let diff = run_diff(&root, false);
    let text = combined_output(&diff);
    assert!(text.contains("New model (not yet deployed)"), "{text}");
    assert!(
        text.contains("state.mode: stateless") && text.contains("no schema snapshots"),
        "the say-so must name the posture and what it excludes: {text}"
    );

    let diff_json = run_diff(&root, true);
    let json_text = String::from_utf8_lossy(&diff_json.stdout).to_string();
    let parsed: serde_json::Value =
        serde_json::from_str(&json_text).unwrap_or_else(|e| panic!("{e}: {json_text}"));
    let model = parsed["models"]
        .as_array()
        .and_then(|models| models.iter().find(|m| m["name"] == "base"))
        .unwrap_or_else(|| panic!("expected a 'base' entry in {json_text}"));
    assert_eq!(model["status"], "new");
    assert!(
        model["snapshot_absent_reason"]
            .as_str()
            .is_some_and(|s| s.contains("stateless")),
        "{json_text}"
    );
}

#[test]
fn diff_reports_new_after_snapshot_deleted() {
    let tmp = TempDir::new().expect("tempdir");
    let root = stage_workspace(&tmp, "intervals");

    let build = run_build(&root);
    assert!(
        build.status.success(),
        "build must succeed: {}",
        combined_output(&build)
    );

    let schema_path = root.join(".smelt/targets/dev/schemas/base.json");
    assert!(
        schema_path.exists(),
        "an `intervals` posture build must write the deployed schema snapshot"
    );
    std::fs::remove_file(&schema_path).expect("delete deployed schema snapshot");

    let diff = run_diff(&root, false);
    let text = combined_output(&diff);
    assert!(text.contains("New model (not yet deployed)"), "{text}");
    assert!(
        !text.contains("state.mode"),
        "a deleted snapshot under a snapshot-writing posture is a plain `new`, \
         not the posture-excluded say-so: {text}"
    );
}
