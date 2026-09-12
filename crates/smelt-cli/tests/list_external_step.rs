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

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
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

/// Criterion 3 on a real fixture, not a synthetic scaffold: `examples/
/// github_activity` declares its day loader (`load_day.sh`) as one external
/// step producing both raw sources (`docs/outcomes/20260906-external-dag-
/// steps/phases/07-plan.md` task 6).
///
/// Goes through `smelt explain --json`, not `smelt list --format json`:
/// `examples/github_activity/{sample,setup_sources}.sql` (and the
/// equivalent root-level `setup_sources.sql` in every other multi-file
/// example) are plain utility scripts, not smelt models, but `load_workspace`
/// discovers them project-wide regardless of `config.paths`
/// (`docs/specs/architecture.md` §"Workspace loading parity") and `smelt
/// list`'s `ListError::ParseErrors` check
/// (`crates/smelt-cli/src/commands/list.rs`) is unconditional over every
/// discovered file, not the selected set — a pre-existing gap, reproduced
/// identically on `examples/web_analytics` and `examples/retail_analytics`,
/// unrelated to external steps and out of this phase's scope. `smelt
/// explain`, like `smelt run`, only compiles what selection actually reaches.
#[test]
fn github_activity_declares_its_loader_step() {
    let project_dir = repo_root().join("examples/github_activity");

    let out = run(&project_dir, &["explain", "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "smelt explain --json should exit 0 over examples/github_activity.\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    let steps = parsed["external_steps"]
        .as_object()
        .unwrap_or_else(|| panic!("expected an external_steps object: {stdout}"));
    assert_eq!(
        steps.len(),
        1,
        "expected exactly one external step declared in examples/github_activity: {steps:?}"
    );
    let step = steps.values().next().unwrap();
    let produces = step["produces"].as_array().expect("produces is an array");
    let produces: Vec<&str> = produces.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        produces.contains(&"smelt.sources.raw.github_events"),
        "expected the step to produce smelt.sources.raw.github_events: {produces:?}"
    );
    assert!(
        produces.contains(&"smelt.sources.raw.github_events_arrival"),
        "expected the step to produce smelt.sources.raw.github_events_arrival: {produces:?}"
    );
}
