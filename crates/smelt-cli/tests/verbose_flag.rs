#![cfg(feature = "duckdb")]
//! Phase 4 of `docs/plans/20260502-smelt-loop-findings.md` (TB-1):
//! `smelt build --verbose` must log compiled SQL for each model immediately
//! before execution. Spec: `docs/specs/cli.md` §"`--verbose`".
//!
//! These tests pin the contract that the verbose flag actually emits
//! per-model SQL on stdout (not via `tracing::debug!`, which is filtered
//! out unless `RUST_LOG=debug` is set).

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_smelt_yml(dir: &Path, name: &str) {
    let yml = format!(
        "name: {name}\n\
         version: 1\n\
         paths:\n  - models\n\
         \
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n"
    );
    std::fs::write(dir.join("smelt.yml"), yml).unwrap();
}

fn stage_workspace(tmp: &TempDir, name: &str, model_files: &[(&str, &str)]) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("seeds")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    write_smelt_yml(&root, name);
    for (file, content) in model_files {
        std::fs::write(root.join("models").join(file), content).unwrap();
    }
    root
}

fn run_build_with_args(project_dir: &Path, extra_args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(smelt_bin());
    cmd.arg("build")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .args(extra_args)
        // No RUST_LOG: --verbose must work without an external log filter.
        .env_remove("RUST_LOG");
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"))
}

/// Test 1 — `smelt build --verbose` must surface the compiled SQL for each
/// executed model on stdout, even with no `RUST_LOG` set. The compiled SQL
/// is the user's primary debugging signal; routing it through `tracing::debug!`
/// (filtered out by default) makes the flag a no-op.
#[test]
fn test_verbose_build_logs_sql() {
    let tmp = TempDir::new().unwrap();
    let workspace = stage_workspace(
        &tmp,
        "verbose_logs_sql_ws",
        &[
            ("base.sql", "SELECT 1 AS x\n"),
            ("derived.sql", "SELECT x + 1 AS y FROM smelt.models.base\n"),
        ],
    );

    let output = run_build_with_args(&workspace, &["--verbose"]);

    assert!(
        output.status.success(),
        "smelt build --verbose must exit 0 (got {:?}); \
         stderr:\n{}\nstdout:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // The compiled SQL for both models must appear on stdout. We check for
    // the SELECT keyword (the SQL is compiled but the SELECT remains) and
    // the model names as a prefix marker. Be lenient on case and whitespace.
    let lower = stdout.to_lowercase();
    assert!(
        lower.contains("select"),
        "stdout must contain compiled SQL with a SELECT keyword; \
         got stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        stdout.contains("base") && stdout.contains("derived"),
        "stdout must mention both model names (base, derived) as part of \
         per-model verbose output; got stdout:\n{stdout}"
    );
}

/// Test 2 — `smelt build` (no `--verbose`) must not dump compiled SQL on
/// stdout. The SELECT keyword must not appear anywhere on stdout, because
/// the verbose flag is the *only* path that emits compiled SQL.
#[test]
fn test_non_verbose_build_no_sql() {
    let tmp = TempDir::new().unwrap();
    let workspace = stage_workspace(
        &tmp,
        "non_verbose_no_sql_ws",
        &[
            ("base.sql", "SELECT 1 AS x\n"),
            ("derived.sql", "SELECT x + 1 AS y FROM smelt.models.base\n"),
        ],
    );

    let output = run_build_with_args(&workspace, &[]);

    assert!(
        output.status.success(),
        "smelt build must exit 0 (got {:?}); \
         stderr:\n{}\nstdout:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lower = stdout.to_lowercase();
    assert!(
        !lower.contains("select"),
        "stdout must NOT contain compiled SQL when --verbose is not set; \
         got stdout:\n{stdout}"
    );
}

/// Test 3 — `smelt build --verbose` must produce no compiled-SQL output for
/// models that are skipped because they are not selected. We use a
/// `tag:` selector matching no models so zero models are executed; the
/// per-executed-model verbose path must therefore stay silent.
///
/// (The original phrasing was "second run sees up-to-date models and
/// skips them", but smelt today does not implement up-to-date detection
/// for view-materialised models — they always re-execute. So instead we
/// exercise the same contract — "no execution = no verbose output" — via
/// an empty selection. The spec rule the test pins is: verbose output is
/// *per executed model*, not per discovered model.)
#[test]
fn test_verbose_no_models_executed_no_sql() {
    let tmp = TempDir::new().unwrap();
    let workspace = stage_workspace(
        &tmp,
        "verbose_no_exec_ws",
        &[
            ("base.sql", "SELECT 1 AS x\n"),
            ("derived.sql", "SELECT x + 1 AS y FROM smelt.models.base\n"),
        ],
    );

    // Selector matches no models (no tags defined) — zero models executed.
    let output = run_build_with_args(
        &workspace,
        &["--verbose", "--select", "tag:nonexistent_tag"],
    );

    assert!(
        output.status.success(),
        "smelt build --verbose --select tag:nonexistent_tag must exit 0 \
         (got {:?}); stderr:\n{}\nstdout:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lower = stdout.to_lowercase();
    assert!(
        !lower.contains("select"),
        "stdout must NOT contain compiled SQL when no models are executed \
         (verbose output is per executed model); got stdout:\n{stdout}"
    );
}
