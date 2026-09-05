//! Integration coverage for `smelt_lsp::property_diff::refresh` that does
//! not need a running server (`docs/outcomes/20260905-property-diff/
//! phases/07-plan.md` tests 7 and 8).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

fn git(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git must be available to run these tests");
    assert!(
        output.status.success(),
        "git {:?} failed in {:?}: {}",
        args,
        dir,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_commit(dir: &Path, message: &str) {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=t",
            "commit",
            "-m",
            message,
        ])
        .current_dir(dir)
        .output()
        .expect("git must be available to run these tests");
    assert!(
        output.status.success(),
        "git commit failed in {:?}: {}",
        dir,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" || name == ".smelt" || name == ".git" {
            continue;
        }
        let dest = dst.join(&name);
        if path.is_dir() {
            copy_dir(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).unwrap();
        }
    }
}

fn stage_timeseries_repo(tmp: &Path) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/timeseries");
    copy_dir(&repo_root, tmp);
    git(tmp, &["init", "-q", "-b", "main"]);
    git(tmp, &["add", "-A"]);
    git_commit(tmp, "initial import of examples/timeseries");
}

/// Test 7: a workspace that is not a git work tree gets no lens/diagnostic
/// and a `Silent` outcome, never `Failed`/a panic.
///
/// *Fails against a broken implementation* that raises the fail-loud
/// reflex for a missing baseline as a hard error: this test's assertion on
/// `RefreshOutcome::Silent` (rather than `Failed` or a panic) catches it.
#[test]
fn non_git_workspace_is_silent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Copy a real project layout but never `git init` it.
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/timeseries");
    copy_dir(&repo_root, tmp.path());

    let outcome = smelt_lsp::property_diff::refresh(tmp.path(), &BTreeMap::new(), None);
    match outcome {
        smelt_lsp::property_diff::RefreshOutcome::Silent(reason) => {
            assert!(
                reason.to_lowercase().contains("git"),
                "silent reason should name the git problem: {reason}"
            );
        }
        smelt_lsp::property_diff::RefreshOutcome::Report { .. } => {
            panic!("a non-git workspace must not produce a diff report")
        }
        smelt_lsp::property_diff::RefreshOutcome::Failed(reason) => {
            panic!("a non-git workspace is not a transient failure, it is Silent: {reason}")
        }
    }
}

/// Test 8: the baseline side is reused across refreshes while the resolved
/// commit is unchanged, and re-derived once it moves
/// (`docs/outcomes/20260905-property-diff/phases/07-plan.md` D2).
///
/// *Fails against a broken implementation* two ways: a cache keyed on
/// project root alone (never re-derives) fails the third assertion below
/// (moving the branch would still return the SAME `Arc`); no cache at all
/// (always re-derives) fails the second assertion (the `Arc` would differ
/// even with no git change at all).
#[test]
fn baseline_is_reused_until_head_moves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());
    // Branch off `main` (at C0) so `HEAD` can move without touching `main`
    // itself — `resolve_baseline`'s default merge-base is against `main`.
    git(tmp.path(), &["checkout", "-q", "-b", "feature"]);

    let outcome1 = smelt_lsp::property_diff::refresh(tmp.path(), &BTreeMap::new(), None);
    let baseline1 = match outcome1 {
        smelt_lsp::property_diff::RefreshOutcome::Report { baseline, .. } => baseline,
        other => panic!(
            "expected a Report outcome on the first refresh: {}",
            debug_kind(&other)
        ),
    };

    // Leg 1: no git change at all — the second refresh must reuse the SAME
    // baseline `Arc` (fails a no-cache-at-all implementation).
    let outcome2 = smelt_lsp::property_diff::refresh(
        tmp.path(),
        &BTreeMap::new(),
        Some(Arc::clone(&baseline1)),
    );
    let baseline2 = match outcome2 {
        smelt_lsp::property_diff::RefreshOutcome::Report { baseline, .. } => baseline,
        other => panic!(
            "expected a Report outcome on the second refresh: {}",
            debug_kind(&other)
        ),
    };
    assert!(
        Arc::ptr_eq(&baseline1, &baseline2),
        "an unchanged commit must reuse the cached baseline side, not re-derive it"
    );

    // Leg 2: commit onto `feature` itself (HEAD moves) — `main` hasn't
    // moved, so `merge-base(feature, main)` is still C0: still reused.
    std::fs::write(tmp.path().join("FEATURE_NOTE.md"), "feature work\n").unwrap();
    git(tmp.path(), &["add", "-A"]);
    git_commit(tmp.path(), "commit on feature, merge-base unaffected");

    let outcome3 = smelt_lsp::property_diff::refresh(
        tmp.path(),
        &BTreeMap::new(),
        Some(Arc::clone(&baseline2)),
    );
    let baseline3 = match outcome3 {
        smelt_lsp::property_diff::RefreshOutcome::Report { baseline, .. } => baseline,
        other => panic!(
            "expected a Report outcome on the third refresh: {}",
            debug_kind(&other)
        ),
    };
    assert!(
        Arc::ptr_eq(&baseline2, &baseline3),
        "a commit on the current branch that doesn't move the merge-base must still reuse the cache"
    );

    // Leg 3: move the baseline branch (`main`) forward to feature's tip —
    // `merge-base(feature, main)` now equals feature's own HEAD, which
    // differs from C0: this refresh must re-derive (fails a cache keyed on
    // project root alone, which never re-derives).
    git(tmp.path(), &["checkout", "-q", "main"]);
    git(tmp.path(), &["merge", "-q", "--ff-only", "feature"]);
    git(tmp.path(), &["checkout", "-q", "feature"]);

    let outcome4 = smelt_lsp::property_diff::refresh(
        tmp.path(),
        &BTreeMap::new(),
        Some(Arc::clone(&baseline3)),
    );
    let baseline4 = match outcome4 {
        smelt_lsp::property_diff::RefreshOutcome::Report { baseline, .. } => baseline,
        other => panic!(
            "expected a Report outcome on the fourth refresh: {}",
            debug_kind(&other)
        ),
    };
    assert!(
        !Arc::ptr_eq(&baseline3, &baseline4),
        "moving the baseline branch forward must re-derive, not reuse the stale cache"
    );
}

fn debug_kind(outcome: &smelt_lsp::property_diff::RefreshOutcome) -> String {
    match outcome {
        smelt_lsp::property_diff::RefreshOutcome::Report { .. } => "Report".to_string(),
        smelt_lsp::property_diff::RefreshOutcome::Silent(r) => format!("Silent({r})"),
        smelt_lsp::property_diff::RefreshOutcome::Failed(r) => format!("Failed({r})"),
    }
}
