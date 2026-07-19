#![cfg(feature = "duckdb")]
//! Integration tests for `smelt init` — the non-interactive project scaffolder.
//!
//! TDD: written before the implementation to drive the feature. Each test
//! runs the real `smelt` binary against a temp directory, following the same
//! harness style as `crates/smelt-cli/tests/check_command.rs`.
//!
//! Spec: `docs/specs/cli.md` §"`smelt init` — non-interactive scaffolder".

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

/// Run `smelt init <dir>` and return the output.
fn run_init(dir: &std::path::Path) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("init")
        .arg(dir)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt init`: {e}"))
}

/// `smelt init` scaffolds a project containing exactly the files the spec
/// promises, and that project builds green against DuckDB with no further
/// edits.
#[test]
fn init_scaffolds_project_that_builds() {
    let tmp = TempDir::new().unwrap();
    let project_dir = tmp.path().join("my-project");

    let out = run_init(&project_dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "smelt init should exit 0 on a fresh directory.\nstdout: {stdout}\nstderr: {stderr}"
    );

    assert!(
        project_dir.join("smelt.yml").is_file(),
        "expected smelt.yml to be scaffolded"
    );
    assert!(
        project_dir.join("models").is_dir(),
        "expected a models/ directory"
    );
    let model_files: Vec<_> = std::fs::read_dir(project_dir.join("models"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "sql"))
        .collect();
    assert_eq!(
        model_files.len(),
        1,
        "expected exactly one example model, found: {:?}",
        model_files.iter().map(|e| e.path()).collect::<Vec<_>>()
    );

    let seed_files: Vec<_> = std::fs::read_dir(project_dir.join("seeds"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "csv"))
        .collect();
    assert_eq!(
        seed_files.len(),
        1,
        "expected exactly one seed CSV, found: {:?}",
        seed_files.iter().map(|e| e.path()).collect::<Vec<_>>()
    );

    let gitignore = std::fs::read_to_string(project_dir.join(".gitignore"))
        .expect("expected a .gitignore to be scaffolded");
    assert!(
        gitignore.contains(".smelt/"),
        ".gitignore should exclude the state directory:\n{gitignore}"
    );
    assert!(
        gitignore.to_lowercase().contains("duckdb"),
        ".gitignore should exclude the database file:\n{gitignore}"
    );

    // The scaffold builds cleanly with no further edits.
    let build_out = Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));
    assert!(
        build_out.status.success(),
        "smelt build should exit 0 against the scaffold.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr),
    );
}

/// `smelt init` refuses to run against a directory that already contains a
/// `smelt.yml`: exit non-zero (spec: exit `2`), message names the conflict,
/// and there is deliberately no `--force` flag to override this.
#[test]
fn init_refuses_nonempty_dir_without_force() {
    let tmp = TempDir::new().unwrap();
    let project_dir = tmp.path().join("existing-project");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(
        project_dir.join("smelt.yml"),
        "name: existing\nversion: 1\n",
    )
    .unwrap();

    let out = run_init(&project_dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "smelt init should refuse a directory that already has smelt.yml.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "smelt init on a conflicting directory is a usage error (exit 2).\nstdout: {stdout}\nstderr: {stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("smelt.yml"),
        "error message should name the conflicting file.\ncombined: {combined}"
    );

    // No --force flag exists to override the refusal.
    let force_out = Command::new(smelt_bin())
        .arg("init")
        .arg(&project_dir)
        .arg("--force")
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt init --force`: {e}"));
    assert!(
        !force_out.status.success(),
        "smelt init has no --force flag; passing one should be a clap usage error"
    );
}

/// The scaffolded workspace loads through the shared `load_workspace` path
/// (workspace-loading parity — CLI ↔ LSP) with zero errors/diagnostics.
#[test]
fn init_scaffold_has_no_diagnostics() {
    let tmp = TempDir::new().unwrap();
    let project_dir = tmp.path().join("clean-project");

    let out = run_init(&project_dir);
    assert!(
        out.status.success(),
        "smelt init should succeed before checking diagnostics.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let loaded = smelt_core::workspace::load_workspace(&project_dir);
    assert!(
        loaded.errors.is_empty(),
        "scaffolded workspace should load with zero diagnostics, got: {:?}",
        loaded.errors
    );
    assert!(
        !loaded.sql_files.is_empty(),
        "scaffolded workspace should discover at least the example model"
    );
}
