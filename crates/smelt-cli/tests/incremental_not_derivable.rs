//! Integration test: a model with a bare LAG (no RANGE BETWEEN INTERVAL) over a
//! timeseries source must cause `smelt run` to exit non-zero with a `NotDerivable`
//! diagnostic.  The `--allow-downgrade` flag must suppress the refusal.
//!
//! Fixture: `examples/timeseries_broken_bare_lag_not_derivable/`
//!   - `events.sql`         — timeseries source (partition_column: event_date)
//!   - `daily_with_lag.sql` — incremental model with bare LAG(amount) OVER (...)
//!     whose bound cannot be derived.

use std::path::{Path, PathBuf};
use std::process::Command;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

/// Path to the broken fixture workspace.
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/timeseries_broken_bare_lag_not_derivable")
}

fn run_smelt_run(project_dir: &Path, extra_flags: &[&str]) -> std::process::Output {
    // Use a per-run temp database so the fixture directory stays clean
    // and the test can run in parallel without conflicts.
    let db_dir = tempfile::TempDir::new().expect("create temp dir");
    let db_path = db_dir.path().join("test.duckdb");

    let mut cmd = Command::new(smelt_bin());
    cmd.args([
        "run",
        "--dry-run",
        "--project-dir",
        project_dir.to_str().unwrap(),
        "--database",
        db_path.to_str().unwrap(),
    ]);
    for flag in extra_flags {
        cmd.arg(flag);
    }
    // Keep db_dir alive until the command completes.
    let out = cmd
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt: {e}"));
    drop(db_dir);
    out
}

/// A model with a bare LAG over a timeseries source must be refused.
///
/// The exact diagnostic text must contain "NotDerivable" or describe the
/// problem (cannot derive / temporal bound / LAG), and the model name must
/// appear.
#[test]
fn test_bare_lag_not_derivable_refused() {
    let dir = fixture_dir();
    let output = run_smelt_run(&dir, &[]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stderr}{stdout}");

    assert!(
        !output.status.success(),
        "smelt run must exit non-zero for a bare-LAG incremental model;\
         \nstderr:\n{stderr}\nstdout:\n{stdout}"
    );

    // Diagnostic must reference the problematic pattern or the refusal mechanism.
    let lower = combined.to_lowercase();
    assert!(
        lower.contains("not derivable")
            || lower.contains("notderivable")
            || lower.contains("cannot derive")
            || lower.contains("temporal bound")
            || lower.contains("lag"),
        "diagnostic must describe the NotDerivable problem; got:\n{combined}"
    );

    // Diagnostic must name the refused model.
    assert!(
        combined.contains("daily_with_lag"),
        "diagnostic must name the refused model 'daily_with_lag'; got:\n{combined}"
    );
}

/// With `--allow-downgrade`, the bare-LAG model must not cause a hard refusal.
#[test]
fn test_bare_lag_allow_downgrade_suppresses_refusal() {
    let dir = fixture_dir();
    let output = run_smelt_run(&dir, &["--allow-downgrade"]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "--allow-downgrade must suppress the NotDerivable refusal;\
         \nstderr:\n{stderr}\nstdout:\n{stdout}"
    );
}
