#![cfg(feature = "duckdb")]
//! Regression gate: the `cumulative_aggregate` classifier must run on the
//! **no-window full-refresh** path, not only on the windowed per-partition
//! merge path.
//!
//! `docs/specs/cumulative_aggregate.md` Constraint #10 — *No silent downgrade*:
//! "A classifier rejection refuses the model at planning time. No fallback to
//! full-refresh, no fallback to incremental, no warning-then-continue."
//!
//! Before the fix, `smelt run`/`smelt build` without `--event-time-start/-end`
//! took a full-refresh shortcut that **bypassed `classify_cumulative`**, so a
//! model using a non-allowlisted aggregator (e.g. `STRING_AGG`) was silently
//! materialised as a plain `CREATE TABLE AS` — exit 0, no diagnostic, wrong
//! contract. This test drives the compiled `smelt` binary (outside-in, the same
//! discipline as `incremental_idempotency.rs`) over a hermetic fixture whose
//! driving source is a self-contained `VALUES` literal (no external seed).

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/cumulative_classifier_gate")
}

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

/// Run `smelt run --select <selector>` (no event-time window) against the
/// fixture in an isolated temp project dir. Returns (exit_success, combined
/// stdout+stderr).
fn run_select(selector: &str) -> (bool, String) {
    // Copy the fixture to a temp dir so the per-test DuckDB file is isolated.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    copy_dir_all(&workspace_dir(), tmp.path()).expect("copy fixture");

    let output = Command::new(smelt_bin())
        .args([
            "run",
            "--project-dir",
            tmp.path().to_str().unwrap(),
            "--select",
            selector,
        ])
        .output()
        .expect("run smelt");

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), combined)
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            // Skip any pre-existing build output.
            if entry.file_name() == "target" {
                continue;
            }
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// A well-formed cumulative model builds on the no-window path. Guards the fix
/// against over-rejection: classifying must not break valid models.
#[test]
fn valid_cumulative_builds_without_window() {
    let (ok, out) = run_select("+edges_valid");
    assert!(
        ok,
        "valid cumulative model must build on the no-window full-refresh path; output:\n{}",
        out
    );
}

/// A cumulative model using a non-allowlisted aggregator (`STRING_AGG`) must be
/// REFUSED on the no-window path, with the `CumulativeUnknownAggregator`
/// diagnostic — not silently materialised as a full refresh.
#[test]
fn unknown_aggregator_refused_without_window() {
    let (ok, out) = run_select("+edges_bad_aggregator");
    assert!(
        !ok,
        "STRING_AGG cumulative must be refused even without a run window \
         (Constraint #10 — no silent downgrade); instead it succeeded. Output:\n{}",
        out
    );
    assert!(
        out.contains("CumulativeUnknownAggregator"),
        "rejection must name the classifier diagnostic CumulativeUnknownAggregator; output:\n{}",
        out
    );
}
