//! Tests for hard refusal of models that fail the incremental safety classifier.
//!
//! A model that declares `incremental:` but whose SQL contains a construct
//! the safety classifier rejects (OVER, HAVING, LIMIT, etc.) must cause
//! `smelt run` to exit non-zero with an explanatory diagnostic.  The previous
//! behaviour was a silent `warn!` followed by a full-table refresh.
//!
//! The `--allow-downgrade` flag restores the full-refresh fallback as an
//! explicit opt-in.

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
         default_materialization: table\n"
    );
    std::fs::write(dir.join("smelt.yml"), yml).unwrap();
}

fn stage_workspace(tmp: &TempDir, name: &str, model_files: &[(&str, &str)]) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join("models")).unwrap();
    write_smelt_yml(&root, name);
    for (file, content) in model_files {
        std::fs::write(root.join("models").join(file), content).unwrap();
    }
    root
}

/// A model with OVER in the outer body (window function — not partition-local).
const SQL_OVER: &str = r#"---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    user_id,
    COUNT(*) as cnt,
    ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_date) as rn
FROM raw.events
GROUP BY 1, 2
"#;

/// A model with HAVING in the outer body.
const SQL_HAVING: &str = r#"---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    user_id,
    COUNT(*) as cnt
FROM raw.events
GROUP BY 1, 2
HAVING COUNT(*) > 0
"#;

/// Run `smelt run --dry-run` with optional extra flags against `project_dir`.
fn run_dry_run_with_flags(project_dir: &Path, extra_flags: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(smelt_bin());
    cmd.args([
        "run",
        "--dry-run",
        "--project-dir",
        project_dir.to_str().unwrap(),
    ]);
    for flag in extra_flags {
        cmd.arg(flag);
    }
    cmd.env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt: {e}"))
}

/// A model with OVER in the outer body is refused at planning time.
#[test]
fn test_outer_over_refused() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "over_refused", &[("bad_over.sql", SQL_OVER)]);
    let output = run_dry_run_with_flags(&ws, &[]);

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    assert!(
        !output.status.success(),
        "smelt run must exit non-zero for a model with OVER that fails safety check; \
         stderr:\n{}\nstdout:\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    // Must mention the rejected construct (OVER or window function)
    let lower = combined.to_lowercase();
    assert!(
        lower.contains("over") || lower.contains("window"),
        "diagnostic must name the rejected construct; got:\n{combined}"
    );

    // Must mention the model name
    assert!(
        combined.contains("bad_over"),
        "diagnostic must name the refused model; got:\n{combined}"
    );
}

/// A model with HAVING in the outer body is refused at planning time.
#[test]
fn test_outer_having_refused() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "having_refused", &[("bad_having.sql", SQL_HAVING)]);
    let output = run_dry_run_with_flags(&ws, &[]);

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    assert!(
        !output.status.success(),
        "smelt run must exit non-zero for a model with HAVING that fails safety check; \
         stderr:\n{}\nstdout:\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    let lower = combined.to_lowercase();
    assert!(
        lower.contains("having"),
        "diagnostic must name the HAVING construct; got:\n{combined}"
    );

    assert!(
        combined.contains("bad_having"),
        "diagnostic must name the refused model; got:\n{combined}"
    );
}

/// A refused model causes a non-zero exit code from `smelt run`.
#[test]
fn test_refusal_exit_nonzero() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "refusal_exit", &[("bad_over.sql", SQL_OVER)]);
    let output = run_dry_run_with_flags(&ws, &[]);

    assert!(
        !output.status.success(),
        "exit code must be non-zero when a model is refused; got {:?}",
        output.status
    );
}

/// With `--allow-downgrade`, planning succeeds (full-table fallback).
///
/// The model still runs — as a full-table refresh — rather than being refused.
/// `--dry-run` is used so no actual DB write is needed.
#[test]
fn test_allow_downgrade_succeeds() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "allow_downgrade", &[("bad_over.sql", SQL_OVER)]);
    let output = run_dry_run_with_flags(&ws, &["--allow-downgrade"]);

    assert!(
        output.status.success(),
        "--allow-downgrade must suppress the hard refusal and exit 0; \
         stderr:\n{}\nstdout:\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}

/// `smelt build --allow-downgrade` is accepted by clap and suppresses refusal.
///
/// This smoke test confirms that `BuildArgs` exposes `--allow-downgrade` and
/// threads it through to the inner `RunArgs`.  Full semantic coverage of the
/// flag's effect (full-table fallback) is in the `smelt run` tests above;
/// here we verify:
///   1. clap does not reject the flag as an "unexpected argument"
///   2. the safety-classifier refusal is suppressed (no "window functions" error)
///
/// `smelt build` seeds then runs models, so execution will fail when the
/// database is absent/empty — but the failure must come from SQL execution,
/// not from planning-time refusal of the incremental model.
#[test]
fn test_build_allow_downgrade_flag_accepted() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "build_allow_downgrade", &[("bad_over.sql", SQL_OVER)]);

    let output = Command::new(smelt_bin())
        .args([
            "build",
            "--project-dir",
            ws.to_str().unwrap(),
            "--allow-downgrade",
        ])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt build: {e}"));

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    // clap must not reject --allow-downgrade as an unknown flag
    assert!(
        !combined.contains("unexpected argument"),
        "`smelt build --allow-downgrade` must be accepted by clap; got:\n{combined}"
    );

    // The safety-classifier refusal must be suppressed — no "window functions" error
    assert!(
        !combined.to_lowercase().contains("window functions"),
        "`smelt build --allow-downgrade` must suppress the window-function refusal; got:\n{combined}"
    );
}

/// Without `--allow-downgrade`, the refused model must NOT be executed.
///
/// We verify by checking that `smelt run` (not dry-run) exits non-zero before
/// any SQL reaches the backend — the table should not be populated.
/// We use a non-existent `raw.events` source; if run were attempted it would
/// fail differently (table not found), but refusal should fire first.
#[test]
fn test_no_full_refresh_fallback() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "no_fallback", &[("bad_over.sql", SQL_OVER)]);

    // Run without --dry-run so execution would be attempted if not refused.
    let mut cmd = Command::new(smelt_bin());
    cmd.args([
        "run",
        "--project-dir",
        ws.to_str().unwrap(),
        "--event-time-start",
        "2024-01-01",
        "--event-time-end",
        "2024-01-02",
    ]);
    cmd.env_remove("RUST_LOG");
    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt: {e}"));

    // Must exit non-zero
    assert!(
        !output.status.success(),
        "smelt run must exit non-zero on refusal, not silently downgrade; \
         stderr:\n{}\nstdout:\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    // Must not have produced a "table not found" error — refusal fires at
    // planning time, before any SQL execution touches the backend.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Table with name events does not exist") && !stderr.contains("raw.events"),
        "error must come from planning-time refusal, not SQL execution; got:\n{stderr}"
    );
}
