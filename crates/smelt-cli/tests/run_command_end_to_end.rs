#![cfg(feature = "duckdb")]
//! Phase 4 of `docs/plans/20260524-cli-runtime-migration.md`:
//! `commands/run.rs` is a thin wrapper over `smelt_runtime::execute_project`.
//!
//! These tests assert the four contracts that are uniquely `run.rs`'s
//! responsibility after the migration:
//!
//!   1. `test_full_refresh_run` — a basic full-refresh run exits 0 and emits
//!      a "built N model(s)" summary.
//!   2. `test_verbose_prints_compiled_sql` — `smelt run -v` prints the compiled
//!      SQL for each model to stdout (via the `RunReporter::model_compiled`
//!      callback added in this phase).
//!   3. `test_allow_downgrade_warns_and_runs` — `smelt run --allow-downgrade`
//!      against a model that would normally fail the incremental safety check
//!      succeeds (exits 0, falling back to full-refresh).
//!   4. `test_show_plan_dry_run` — `smelt run --dry-run --show-plan` prints an
//!      "Execution plan:" section listing each model's resolved strategy.

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
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n"
    );
    std::fs::write(dir.join("smelt.yml"), yml).unwrap();
}

fn stage_workspace(tmp: &TempDir, name: &str, model_files: &[(&str, &str)]) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    write_smelt_yml(&root, name);
    for (file, content) in model_files {
        std::fs::write(root.join("models").join(file), content).unwrap();
    }
    root
}

fn run_smelt(project_dir: &Path, extra_args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(smelt_bin());
    cmd.args(["run", "--project-dir", project_dir.to_str().unwrap()])
        .args(extra_args)
        .env_remove("RUST_LOG");
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

/// A simple view-materialised model — safe for any run mode.
const SQL_SIMPLE: &str = "SELECT 1 AS x, 2 AS y\n";

/// An incremental model with OVER in the outer body — the safety classifier
/// rejects it unless `--allow-downgrade` is passed.
const SQL_OVER_INCREMENTAL: &str = r#"---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    user_id,
    COUNT(*) AS cnt,
    ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_date) AS rn
FROM raw.events
GROUP BY 1, 2
"#;

/// 1. A full-refresh run against a simple fixture exits 0 and emits the
///    "built N model(s)" summary on stderr.
///
/// This pins the fundamental contract of `commands/run.rs` after the Phase 4
/// migration: the command is a thin wrapper over `execute_project` — the
/// business logic lives in the runtime and `run.rs` only provides arg parsing,
/// a reporter, and a backend factory.
#[test]
fn test_full_refresh_run() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "run_full_refresh_ws", &[("simple.sql", SQL_SIMPLE)]);

    let output = run_smelt(&ws, &[]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "smelt run must exit 0 for a simple view; \
         stderr:\n{stderr}\nstdout:\n{stdout}"
    );
    assert!(
        stderr.contains("built 1 model(s)"),
        "smelt run must emit the 'built N model(s)' summary to stderr; \
         got stderr:\n{stderr}"
    );
}

/// 2. `smelt run -v` emits compiled SQL to stdout via the `model_compiled`
///    reporter callback. The SQL must appear without RUST_LOG being set — the
///    original bug was that verbose output went through tracing::debug!, which
///    is filtered out by default.
#[test]
fn test_verbose_prints_compiled_sql() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(
        &tmp,
        "run_verbose_ws",
        &[("simple.sql", SQL_SIMPLE)],
    );

    let output = run_smelt(&ws, &["--verbose"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "smelt run --verbose must exit 0; \
         stderr:\n{stderr}\nstdout:\n{stdout}"
    );

    let lower = stdout.to_lowercase();
    assert!(
        lower.contains("select"),
        "smelt run --verbose must print compiled SQL (containing SELECT) to stdout; \
         got stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("simple"),
        "stdout must contain the model name as a header; got:\n{stdout}"
    );
}

/// 3. `smelt run --allow-downgrade --dry-run` against an unsafe incremental
///    model (one with OVER in the outer body, which the safety classifier
///    rejects) exits 0. The `--allow-downgrade` flag sets
///    `ExecuteRequest::enforce_safety = false`, bypassing the refusal.
///
///    Uses `--dry-run` to avoid needing `raw.events` to exist in DuckDB — we
///    only care that the safety gate doesn't fire, not that execution succeeds.
#[test]
fn test_allow_downgrade_warns_and_runs() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(
        &tmp,
        "run_allow_downgrade_ws",
        &[("bad_over.sql", SQL_OVER_INCREMENTAL)],
    );

    // Without --allow-downgrade: safety check fires, exits non-zero.
    let refused = run_smelt(&ws, &["--dry-run"]);
    assert!(
        !refused.status.success(),
        "smelt run --dry-run without --allow-downgrade must fail for an unsafe incremental; \
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr),
    );

    // With --allow-downgrade: safety check is bypassed, exits 0.
    let allowed = run_smelt(&ws, &["--dry-run", "--allow-downgrade"]);
    let allowed_stderr = String::from_utf8_lossy(&allowed.stderr);
    let allowed_stdout = String::from_utf8_lossy(&allowed.stdout);
    assert!(
        allowed.status.success(),
        "smelt run --dry-run --allow-downgrade must exit 0 for an unsafe incremental model; \
         stderr:\n{allowed_stderr}\nstdout:\n{allowed_stdout}"
    );
}

/// 4. `smelt run --dry-run --show-plan` prints an "Execution plan:" section
///    followed by each model's name and resolved strategy. This pins the
///    PlanSummary-to-stdout rendering that `commands/run.rs` owns (the runtime
///    produces the `PlanSummary`; run.rs formats it).
#[test]
fn test_show_plan_dry_run() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(
        &tmp,
        "run_show_plan_ws",
        &[
            ("alpha.sql", SQL_SIMPLE),
            ("beta.sql", "SELECT x + 1 AS z FROM smelt.alpha\n"),
        ],
    );

    let output = run_smelt(&ws, &["--dry-run", "--show-plan"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "smelt run --dry-run --show-plan must exit 0; \
         stderr:\n{stderr}\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("Execution plan:"),
        "stdout must contain 'Execution plan:' header; got:\n{stdout}"
    );
    assert!(
        stdout.contains("alpha") && stdout.contains("beta"),
        "stdout must list both model names in the execution plan; got:\n{stdout}"
    );
}
