use std::fs;
use std::process::Command;

use crate::{repo_root, script_path};

/// The committed tree must exactly match the baseline — two-sided ratchet.
/// Also verifies that a regression (extra .unwrap() injected) is detected.
#[test]
fn gate_detects_regression() {
    let root = repo_root();
    let script = script_path();

    assert!(
        script.exists(),
        "hardening-budget.sh not found at {script:?} — Phase 1 not yet implemented"
    );

    // ── Test A: committed tree exits 0 ────────────────────────────────────────
    // If this fails, either the tree regressed (count > baseline) or the
    // baseline is stale because debt was paid (count < baseline).
    let status = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", &root)
        .current_dir(&root)
        .status()
        .expect("failed to run hardening-budget.sh");
    assert!(
        status.success(),
        "hardening-budget.sh failed on committed tree.\n\
         • If production debt grew: revert or fix the regression.\n\
         • If debt shrank (good!): run `.claude/scripts/hardening-budget.sh --update` \
           to tighten the baseline."
    );

    // ── Test B: injected .unwrap() is detected ────────────────────────────────
    let tempdir = tempfile::tempdir().unwrap();
    let fake_root = tempdir.path();

    // Minimal fake crate with one .unwrap()
    let fake_src = fake_root.join("crates/smelt-hardening-probe/src");
    fs::create_dir_all(&fake_src).unwrap();
    fs::write(
        fake_src.join("lib.rs"),
        "pub fn probe() -> i32 { let x: Option<i32> = None; x.unwrap() }\n",
    )
    .unwrap();

    // Baseline claims 0 unwrap → the injected one is a regression
    let fake_claude = fake_root.join(".claude");
    fs::create_dir_all(&fake_claude).unwrap();
    fs::write(
        fake_claude.join("hardening-baseline.txt"),
        "smelt-hardening-probe unwrap 0\n\
         smelt-hardening-probe expect 0\n\
         smelt-hardening-probe println 0\n",
    )
    .unwrap();

    let status = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", fake_root)
        .current_dir(&root)
        .status()
        .expect("failed to run hardening-budget.sh on fake tree");
    assert!(
        !status.success(),
        "hardening-budget.sh should have detected the injected regression but exited 0"
    );
}
