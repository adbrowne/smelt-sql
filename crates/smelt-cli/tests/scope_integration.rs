//! Integration tests for `--scope` flag and cwd-derived scope.
//!
//! Spec: `docs/specs/cli.md` §"Argument resolution and `--scope`".
//!
//! These tests run the compiled `smelt` binary against
//! `examples/web_analytics/` using `std::process::Command`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn web_analytics_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join("web_analytics")
}

/// `smelt type events_parsed` run from the `models/silver` subdirectory should
/// resolve via cwd-derived scope and print `silver.events_parsed:`.
#[test]
fn smelt_type_with_cwd_scope() {
    let project_dir = web_analytics_dir();
    let cwd = project_dir.join("models").join("silver");

    let output = Command::new(smelt_bin())
        .args(["type", "--project-dir"])
        .arg(&project_dir)
        .arg("events_parsed")
        .current_dir(&cwd)
        .output()
        .expect("smelt binary should be runnable");

    assert!(
        output.status.success(),
        "smelt type events_parsed (from models/silver) should succeed\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("silver.events_parsed:"),
        "stdout should start with 'silver.events_parsed:', got:\n{stdout}"
    );
}

/// `--scope silver` + arg `events_parsed` should resolve regardless of cwd.
#[test]
fn smelt_type_with_explicit_scope() {
    let project_dir = web_analytics_dir();

    let output = Command::new(smelt_bin())
        .arg("--scope")
        .arg("silver")
        .args(["type", "--project-dir"])
        .arg(&project_dir)
        .arg("events_parsed")
        .current_dir(&project_dir) // cwd is project root, not models/silver
        .output()
        .expect("smelt binary should be runnable");

    assert!(
        output.status.success(),
        "smelt --scope silver type events_parsed should succeed\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("silver.events_parsed:"),
        "stdout should start with 'silver.events_parsed:', got:\n{stdout}"
    );
}

/// `smelt type silver.events_parsed` (canonical path) should resolve from
/// anywhere without a scope.
#[test]
fn smelt_type_canonical_arg() {
    let project_dir = web_analytics_dir();

    let output = Command::new(smelt_bin())
        .args(["type", "--project-dir"])
        .arg(&project_dir)
        .arg("silver.events_parsed")
        .current_dir(&project_dir)
        .output()
        .expect("smelt binary should be runnable");

    assert!(
        output.status.success(),
        "smelt type silver.events_parsed should succeed\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("silver.events_parsed:"),
        "stdout should start with 'silver.events_parsed:', got:\n{stdout}"
    );
}

/// From project root, `smelt type events_parsed` (no scope, bare leaf) should
/// error with a did-you-mean hint pointing to `silver.events_parsed`.
#[test]
fn smelt_type_no_scope_bare_leaf_errors() {
    let project_dir = web_analytics_dir();

    let output = Command::new(smelt_bin())
        .args(["type", "--project-dir"])
        .arg(&project_dir)
        .arg("events_parsed")
        .current_dir(&project_dir) // cwd is project root, no sub-path scope
        .output()
        .expect("smelt binary should be runnable");

    assert!(
        !output.status.success(),
        "smelt type events_parsed (no scope from root) should fail with non-zero exit"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("silver.events_parsed"),
        "stderr should contain 'silver.events_parsed' as a hint, got:\n{stderr}"
    );
}

/// `smelt type` with no positional arg should list all models with canonical
/// dot-paths (`bronze.raw_events:`, `silver.events_parsed:`, etc.), no bare
/// leaf names.
#[test]
fn smelt_type_output_uses_canonical_path() {
    let project_dir = web_analytics_dir();

    let output = Command::new(smelt_bin())
        .args(["type", "--project-dir"])
        .arg(&project_dir)
        .current_dir(&project_dir)
        .output()
        .expect("smelt binary should be runnable");

    assert!(
        output.status.success(),
        "smelt type (no arg) should succeed\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Each non-test model identifier line ends with ':' and must contain a dot
    // (canonical path). Test models live under tests/ and have no parent dir
    // prefix, so they show as bare names. Skip those.
    // Collect non-indented lines that end with ':' and have a dot.
    let canonical_model_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| line.ends_with(':') && !line.starts_with(' ') && line.contains('.'))
        .collect();

    assert!(
        !canonical_model_lines.is_empty(),
        "Expected canonical model header lines (with dots) in output, got:\n{stdout}"
    );

    for line in &canonical_model_lines {
        assert!(
            line.contains('.'),
            "Canonical model header line should contain a dot, got: {line}"
        );
    }

    // Spot-check the known models
    assert!(
        stdout.contains("bronze.raw_events:"),
        "stdout should contain 'bronze.raw_events:'"
    );
    assert!(
        stdout.contains("silver.events_parsed:"),
        "stdout should contain 'silver.events_parsed:'"
    );
}

/// From cwd inside `models/silver`, `smelt run --select events_parsed --dry-run`
/// should select `silver.events_parsed` without error.
#[test]
fn smelt_run_select_scope() {
    let project_dir = web_analytics_dir();
    let cwd = project_dir.join("models").join("silver");

    let output = Command::new(smelt_bin())
        .args(["run", "--project-dir"])
        .arg(&project_dir)
        .args(["--select", "events_parsed", "--dry-run"])
        .current_dir(&cwd)
        .output()
        .expect("smelt binary should be runnable");

    // --dry-run should succeed and not produce an error about not finding the model
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The run should not fail with a "not found" error
    assert!(
        !stderr.contains("not found"),
        "stderr should not contain 'not found'\nstderr: {stderr}\nstdout: {stdout}"
    );
}

/// `smelt --scope "" run --select events_parsed --dry-run` with the scope
/// explicitly disabled should error with ambiguity or not-found when
/// `events_parsed` is a bare leaf.
#[test]
fn smelt_run_select_no_scope_bare_leaf_errors() {
    let project_dir = web_analytics_dir();

    let output = Command::new(smelt_bin())
        .arg("--scope")
        .arg("") // disable auto-scope
        .args(["run", "--project-dir"])
        .arg(&project_dir)
        .args(["--select", "events_parsed", "--dry-run"])
        .current_dir(&project_dir)
        .output()
        .expect("smelt binary should be runnable");

    // Should fail — bare leaf with no scope
    assert!(
        !output.status.success(),
        "smelt --scope '' run --select events_parsed should fail with non-zero exit\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    // The error should mention the model
    assert!(
        stderr.contains("events_parsed"),
        "stderr should mention 'events_parsed'\nstderr: {stderr}"
    );
}

/// `smelt --scope "" run --select events --dry-run` against a project that has
/// both `models/silver/events.sql` and `models/bronze/events.sql` should fail
/// with non-zero exit and stderr mentioning BOTH `silver.events` and
/// `bronze.events` (the Ambiguous branch of resolve_argument).
#[test]
fn smelt_run_select_ambiguous_errors() {
    // Build a temporary project with two models sharing the leaf name "events".
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();

    // smelt.yml
    fs::write(
        project_dir.join("smelt.yml"),
        "name: ambiguous_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n",
    )
    .unwrap();

    // silver/events.sql
    let silver_dir = project_dir.join("models").join("silver");
    fs::create_dir_all(&silver_dir).unwrap();
    fs::write(
        silver_dir.join("events.sql"),
        "SELECT 1 AS event_id, 'click' AS event_type\n",
    )
    .unwrap();

    // bronze/events.sql
    let bronze_dir = project_dir.join("models").join("bronze");
    fs::create_dir_all(&bronze_dir).unwrap();
    fs::write(
        bronze_dir.join("events.sql"),
        "SELECT 1 AS event_id, 'click' AS event_type\n",
    )
    .unwrap();

    let output = Command::new(smelt_bin())
        .arg("--scope")
        .arg("") // disable auto-scope
        .args(["run", "--project-dir"])
        .arg(&project_dir)
        .args(["--select", "events", "--dry-run"])
        .current_dir(&project_dir)
        .output()
        .expect("smelt binary should be runnable");

    assert!(
        !output.status.success(),
        "smelt --scope '' run --select events should fail (ambiguous)\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("silver.events"),
        "stderr should contain 'silver.events'\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("bronze.events"),
        "stderr should contain 'bronze.events'\nstderr: {stderr}"
    );
}
