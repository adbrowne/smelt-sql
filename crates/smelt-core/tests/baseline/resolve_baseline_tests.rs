use std::process::Command;

use smelt_core::baseline::{resolve_baseline, BaselineError, ResolvedAs};

use crate::fixtures::{fixture_repo, git, git_commit, lock};

#[test]
fn resolve_baseline_rejects_non_git_directory() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    let err = resolve_baseline(dir.path(), Some("HEAD")).expect_err("plain dir is not a git repo");
    assert!(
        matches!(err, BaselineError::NotAGitWorkTree { .. }),
        "{err:?}"
    );
}

#[test]
fn resolve_baseline_explicit_ref_resolves_to_commit() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved =
        resolve_baseline(repo.path(), Some("HEAD")).expect("HEAD must resolve in a fresh repo");
    assert_eq!(resolved.resolved_as, ResolvedAs::Explicit);
    assert_eq!(resolved.requested, "HEAD");

    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo.path())
        .output()
        .expect("git rev-parse");
    let expected = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(resolved.commit, expected);
}

#[test]
fn resolve_baseline_unknown_ref_is_an_error() {
    let _guard = lock();
    let repo = fixture_repo();
    let err = resolve_baseline(repo.path(), Some("nope/zzz"))
        .expect_err("nonexistent ref must not resolve");
    assert!(matches!(err, BaselineError::UnknownRef { .. }), "{err:?}");
}

#[test]
fn resolve_baseline_defaults_to_merge_base_with_main() {
    let _guard = lock();
    let repo = fixture_repo();
    git(repo.path(), &["checkout", "-q", "-b", "feature"]);
    std::fs::write(repo.path().join("models/n.sql"), "SELECT 1\n").expect("write model");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "feature commit");

    let resolved = resolve_baseline(repo.path(), None).expect("default baseline must resolve");
    assert_eq!(resolved.resolved_as, ResolvedAs::MergeBase);
    assert_eq!(resolved.requested, "merge-base(main)");

    let output = Command::new("git")
        .args(["merge-base", "HEAD", "main"])
        .current_dir(repo.path())
        .output()
        .expect("git merge-base");
    let expected = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(resolved.commit, expected);
}

#[test]
fn resolve_baseline_falls_back_to_master() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "master"]);
    std::fs::write(dir.path().join("smelt.yml"), "name: fixture\n").expect("write smelt.yml");
    std::fs::create_dir_all(dir.path().join("models")).expect("mkdir models");
    std::fs::write(dir.path().join("models/m.sql"), "SELECT 1\n").expect("write model");
    git(dir.path(), &["add", "-A"]);
    git_commit(dir.path(), "initial");

    git(dir.path(), &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.path().join("models/n.sql"), "SELECT 2\n").expect("write model");
    git(dir.path(), &["add", "-A"]);
    git_commit(dir.path(), "feature commit");

    let resolved = resolve_baseline(dir.path(), None).expect("master fallback must resolve");
    assert_eq!(resolved.resolved_as, ResolvedAs::MergeBase);
    assert_eq!(resolved.requested, "merge-base(master)");
}

#[test]
fn resolve_baseline_errors_when_project_absent_at_ref() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("README.md"), "no project here yet\n").expect("write readme");
    git(dir.path(), &["add", "-A"]);
    git_commit(dir.path(), "initial, no project");

    let project_dir = dir.path().join("sub");
    std::fs::create_dir_all(&project_dir).expect("mkdir sub");
    std::fs::write(project_dir.join("smelt.yml"), "name: fixture\n").expect("write smelt.yml");
    // Uncommitted: the project subdir exists only in the working tree.

    let err = resolve_baseline(&project_dir, Some("HEAD"))
        .expect_err("baseline commit has no project at this path");
    assert!(
        matches!(err, BaselineError::NoProjectAtRef { .. }),
        "{err:?}"
    );
}
