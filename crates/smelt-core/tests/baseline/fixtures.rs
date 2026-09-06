//! Shared git-repo fixtures and low-level `git` helpers for the
//! `baseline` integration tests.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Serializes tests that snapshot the shared OS temp directory
/// (`std::env::temp_dir()`) or `.git/index` metadata — both are process-
/// (or even machine-) wide state that other tests in this binary also
/// touch, so those specific assertions need exclusive access to be
/// deterministic.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// `git`, but with the read-only query flag set so status/list commands
/// used for *assertions* don't themselves perturb `.git/index` — the same
/// hygiene `run_git` in `src/baseline/git.rs` applies to the library's own
/// invocations, needed here so the before/after comparison in
/// `diff_leaves_no_repository_state` is a fair test of the library rather
/// than of the test harness's own queries.
pub(crate) fn git_query(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .expect("git must be available to run these tests")
}

pub(crate) fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git must be available to run these tests");
    assert!(
        output.status.success(),
        "git {:?} failed in {:?}",
        args,
        dir
    );
}

pub(crate) fn git_commit(dir: &Path, message: &str) {
    let output = Command::new("git")
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
    assert!(output.status.success(), "git commit failed in {:?}", dir);
}

/// A minimal single-project git repo: `smelt.yml`, one model, committed on
/// a `main` branch. Returns the `TempDir` (repo root == project dir).
pub(crate) fn fixture_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("smelt.yml"), "name: fixture\n").expect("write smelt.yml");
    std::fs::create_dir_all(dir.path().join("models")).expect("mkdir models");
    std::fs::write(
        dir.path().join("models/m.sql"),
        "SELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id\n",
    )
    .expect("write model");
    git(dir.path(), &["add", "-A"]);
    git_commit(dir.path(), "initial");
    dir
}

/// A repo with enough committed files that `git archive`'s output is a
/// long stream — the shape that makes the pipe-drain bug below observable.
pub(crate) fn fixture_repo_wide() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("smelt.yml"), "name: fixture\n").expect("write smelt.yml");
    std::fs::create_dir_all(dir.path().join("models")).expect("mkdir models");
    for i in 0..200 {
        std::fs::write(
            dir.path().join(format!("models/m{i}.sql")),
            format!("-- {}\nSELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id\n", "x".repeat(400)),
        )
        .expect("write model");
    }
    git(dir.path(), &["add", "-A"]);
    git_commit(dir.path(), "initial");
    dir
}
