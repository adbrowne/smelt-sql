//! Pins the fail-loud preflight of `scripts/bigquery-conformance.sh`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 6).
//!
//! The generative maintenance-conformance BigQuery leg is slow and
//! rate-limited, so a sweep that starts without a valid credential dies
//! mid-case having burned real quota and proved nothing. The script must
//! refuse to start in that case, naming what is missing and the command that
//! fixes it, rather than silently skipping green (which
//! `maintenance_conformance_bigquery`'s own `#[test]`s do — that's the right
//! behaviour for the test binary but the wrong one for a script meant to run
//! a whole sweep unattended) or dying partway through a live run.
//!
//! No `bigquery` feature required and no network touched: `SMELT_BQ_CONFIG_DIR`
//! points at an empty temp directory, so the script's own token-file check
//! fails before it ever reaches `cargo test` or the network.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

fn run_script(config_dir: &Path) -> std::process::Output {
    Command::new("bash")
        .arg(repo_root().join("scripts/bigquery-conformance.sh"))
        .env("SMELT_BQ_CONFIG_DIR", config_dir)
        // Isolate from any real project/token this developer's shell may
        // already have exported — the test asserts the script's OWN
        // detection, not the ambient environment's.
        .env_remove("SMELT_BQ_PROJECT")
        .env_remove("SMELT_BQ_ACCESS_TOKEN")
        .env_remove("SMELT_BQ_DATASET")
        .env_remove("SMELT_BQ_LOCATION")
        .current_dir(repo_root())
        .output()
        .expect("spawn bigquery-conformance.sh")
}

/// An empty config directory has neither a token file nor a project config —
/// the script must refuse to start a sweep, not attempt one and fail mid-run.
#[test]
fn refuses_to_start_with_no_token_present() {
    let tmp = TempDir::new().expect("tempdir");

    let out = run_script(tmp.path());

    assert!(
        !out.status.success(),
        "script must exit non-zero with no token present"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no valid BigQuery access token"),
        "stderr should name the missing token, got:\n{stderr}"
    );
    assert!(
        stderr.contains("bigquery-auth.sh"),
        "stderr should name the fixing command, got:\n{stderr}"
    );
}

/// A present, unexpired token but no `SMELT_BQ_PROJECT` is the silent-skip
/// trap this script exists to prevent: every test in
/// `maintenance_conformance_bigquery` skips green without a project, so a
/// sweep run this way would report success while covering nothing.
#[test]
fn refuses_to_start_with_a_token_but_no_project() {
    let tmp = TempDir::new().expect("tempdir");
    let future_expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_secs()
        + 3000;
    std::fs::write(
        tmp.path().join("token"),
        format!("fake-token-not-used-over-the-network\n{future_expiry}\n"),
    )
    .expect("write fake token file");

    let out = run_script(tmp.path());

    assert!(
        !out.status.success(),
        "script must exit non-zero with no SMELT_BQ_PROJECT configured"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("SMELT_BQ_PROJECT"),
        "stderr should name the missing project, got:\n{stderr}"
    );
}
