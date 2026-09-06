//! `smelt_core::baseline` — git baseline materialisation for the property
//! diff (`docs/specs/property_diff.md` §"Baseline materialisation",
//! `docs/outcomes/20260905-property-diff/phases/04-plan.md`).

use std::path::Path;
use std::process::Command;

use smelt_core::baseline::{edited_set, materialize, resolve_baseline, BaselineError, ResolvedAs};
use smelt_core::sources::discover_source_infos;
use smelt_core::workspace::load_workspace;
use tempfile::TempDir;

/// Serializes tests that snapshot the shared OS temp directory
/// (`std::env::temp_dir()`) or `.git/index` metadata — both are process-
/// (or even machine-) wide state that other tests in this binary also
/// touch, so those specific assertions need exclusive access to be
/// deterministic.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// `git`, but with the read-only query flag set so status/list commands
/// used for *assertions* don't themselves perturb `.git/index` — the same
/// hygiene `run_git` in `src/baseline.rs` applies to the library's own
/// invocations, needed here so the before/after comparison in
/// `diff_leaves_no_repository_state` is a fair test of the library rather
/// than of the test harness's own queries.
fn git_query(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .expect("git must be available to run these tests")
}

fn git(dir: &Path, args: &[&str]) {
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

fn git_commit(dir: &Path, message: &str) {
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
fn fixture_repo() -> TempDir {
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
fn fixture_repo_wide() -> TempDir {
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

/// Regression (#194): `materialize` must not race `git archive` to a
/// broken pipe.
///
/// `tar::Archive::unpack` stops reading at the end-of-archive marker, but
/// `git archive` still has trailing block padding to write. If the read end
/// is dropped first, git dies of `SIGPIPE` and `materialize` reports
/// "`git archive` failed" with an EMPTY stderr — intermittently, and only
/// when the machine is loaded enough for git to still be writing.
#[test]
fn materialize_is_not_racing_git_archive_to_a_broken_pipe() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 25;

    let repo = fixture_repo_wide();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("HEAD resolves");

    // The race only opens when `git` is slow enough to still be writing its
    // trailing padding after `tar` has stopped reading, so the test has to
    // supply the CPU contention itself rather than wait for a loaded CI box.
    let stop = std::sync::atomic::AtomicBool::new(false);

    std::thread::scope(|scope| {
        for _ in 0..(std::thread::available_parallelism().map_or(8, |n| n.get())) {
            let stop = &stop;
            scope.spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    std::hint::spin_loop();
                }
            });
        }

        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                let resolved = &resolved;
                scope.spawn(move || {
                    for _ in 0..ROUNDS {
                        let checkout = materialize(resolved).unwrap_or_else(|e| {
                            panic!("materialize must not fail under concurrency: {e}")
                        });
                        assert!(checkout.project_root().join("models/m0.sql").is_file());
                    }
                })
            })
            .collect();

        let outcome: Result<(), _> = workers
            .into_iter()
            .map(|w| w.join())
            .collect::<Result<Vec<_>, _>>()
            .map(|_| ());
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        outcome.expect("no materialize may lose the race to git archive");
    });
}

// --- resolve_baseline ---

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

// --- materialize ---

#[test]
fn materialize_exports_project_subtree_at_ref() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");

    // Edit the model after the commit — the baseline copy must still hold
    // the committed text.
    std::fs::write(repo.path().join("models/m.sql"), "SELECT 1 -- edited\n").expect("edit model");

    let checkout = materialize(&resolved).expect("materialize");
    let baseline_model =
        std::fs::read_to_string(checkout.project_root().join("models/m.sql")).expect("read");
    assert!(
        baseline_model.contains("GROUP BY customer_id"),
        "baseline copy must hold the committed text, got: {baseline_model}"
    );
    assert!(checkout.project_root().join("smelt.yml").exists());
}

#[test]
fn materialize_of_a_subdirectory_project() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(dir.path().join("proj/models")).expect("mkdir");
    std::fs::write(dir.path().join("proj/smelt.yml"), "name: fixture\n").expect("write");
    std::fs::write(dir.path().join("proj/models/m.sql"), "SELECT 1\n").expect("write");
    std::fs::write(dir.path().join("README.md"), "repo root file\n").expect("write");
    git(dir.path(), &["add", "-A"]);
    git_commit(dir.path(), "initial");

    let project_dir = dir.path().join("proj");
    let resolved = resolve_baseline(&project_dir, Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    assert!(checkout.project_root().join("smelt.yml").exists());
    assert!(checkout.project_root().join("models/m.sql").exists());
    assert!(
        !checkout.project_root().join("README.md").exists(),
        "extracted root must be the project dir's content, not the repo's"
    );
}

#[test]
fn materialize_drops_committed_dot_smelt() {
    let _guard = lock();
    let repo = fixture_repo();
    std::fs::create_dir_all(repo.path().join(".smelt")).expect("mkdir .smelt");
    std::fs::write(repo.path().join(".smelt/x.json"), "{}").expect("write");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "commit .smelt");

    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");
    assert!(
        !checkout.project_root().join(".smelt").exists(),
        "committed .smelt/ must be scrubbed from the baseline copy"
    );
}

#[test]
fn checkout_scratch_is_deleted_on_drop() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");
    let project_root = checkout.project_root().to_path_buf();
    drop(checkout);
    assert!(!project_root.exists(), "scratch must be gone after drop");
}

#[test]
fn checkout_scratch_is_deleted_when_materialization_fails() {
    let _guard = lock();
    let repo = fixture_repo();
    let mut resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    // Corrupt the resolved commit so `git archive` fails after the scratch
    // directory has already been created.
    resolved.commit = "0000000000000000000000000000000000000000".to_string();

    let before: std::collections::BTreeSet<_> = std::fs::read_dir(std::env::temp_dir())
        .expect("read temp dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("smelt-baseline-")
        })
        .map(|e| e.path())
        .collect();

    let err = materialize(&resolved).expect_err("bogus commit must fail materialize");
    assert!(matches!(err, BaselineError::Archive { .. }), "{err:?}");

    let after: std::collections::BTreeSet<_> = std::fs::read_dir(std::env::temp_dir())
        .expect("read temp dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("smelt-baseline-")
        })
        .map(|e| e.path())
        .collect();

    assert_eq!(
        before, after,
        "no smelt-baseline-* scratch entry may survive a failed materialize"
    );
}

#[test]
fn diff_leaves_no_repository_state() {
    let _guard = lock();
    let repo = fixture_repo();
    let first_commit = {
        let out = git_query(repo.path(), &["rev-parse", "HEAD"]);
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    // A second commit so the baseline resolved below is a genuinely EARLIER
    // commit, not `HEAD` — a `checkout`/`reset` to it would be visible.
    std::fs::write(repo.path().join("models/n.sql"), "SELECT 2\n").expect("write model");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "second commit");
    // Dirty the working tree with an uncommitted edit, so a stray
    // `checkout`/`stash` that discarded it would change `git status
    // --porcelain` between the before/after snapshots below — on a clean
    // tree at `HEAD`, that same mutation would be an invisible no-op and
    // this test would pass regardless of whether Constraint 8 held.
    std::fs::write(repo.path().join("models/m.sql"), "SELECT 1 -- dirty edit\n")
        .expect("dirty edit");

    let status_before = git_query(repo.path(), &["status", "--porcelain"]);
    let worktree_before = git_query(repo.path(), &["worktree", "list"]);
    let stash_before = git_query(repo.path(), &["stash", "list"]);
    let refs_before = git_query(repo.path(), &["for-each-ref"]);
    let index_meta_before = std::fs::metadata(repo.path().join(".git/index")).ok();

    let resolved = resolve_baseline(repo.path(), Some(&first_commit)).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");
    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let _ = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);
    drop(checkout);

    let status_after = git_query(repo.path(), &["status", "--porcelain"]);
    let worktree_after = git_query(repo.path(), &["worktree", "list"]);
    let stash_after = git_query(repo.path(), &["stash", "list"]);
    let refs_after = git_query(repo.path(), &["for-each-ref"]);
    let index_meta_after = std::fs::metadata(repo.path().join(".git/index")).ok();

    assert_eq!(status_before.stdout, status_after.stdout);
    assert_eq!(worktree_before.stdout, worktree_after.stdout);
    assert_eq!(stash_before.stdout, stash_after.stdout);
    assert_eq!(refs_before.stdout, refs_after.stdout);
    match (index_meta_before, index_meta_after) {
        (Some(b), Some(a)) => {
            assert_eq!(b.len(), a.len(), ".git/index length must be unchanged");
            assert_eq!(
                b.modified().ok(),
                a.modified().ok(),
                ".git/index mtime must be unchanged"
            );
        }
        (None, None) => {}
        _ => panic!(".git/index existence changed"),
    }
}

/// Structural guard for Constraint 8 ("no repository mutation"): the
/// module's own source contains no mutating git subcommand literal.
#[test]
fn git_surface_uses_no_mutating_subcommand() {
    let _guard = lock();
    let src = include_str!("../src/baseline.rs");
    for banned in [
        "\"checkout\"",
        "\"worktree\"",
        "\"stash\"",
        "\"read-tree\"",
        "\"update-ref\"",
        "\"commit\"",
    ] {
        assert!(
            !src.contains(banned),
            "baseline.rs must not use the mutating git subcommand {banned}"
        );
    }
}

// --- edited_set ---

#[test]
fn edited_set_flags_uncommitted_sql_edit() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(
        repo.path().join("models/m.sql"),
        "SELECT customer_id, MAX(amount) AS total FROM orders GROUP BY customer_id\n",
    )
    .expect("edit without committing");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(set.names.contains("m"), "names={:?}", set.names);
    assert!(
        set.files.iter().any(|f| f.ends_with("models/m.sql")),
        "files={:?}",
        set.files
    );
}

#[test]
fn edited_set_ignores_a_formatting_only_edit() {
    let _guard = lock();
    let repo = fixture_repo();

    // Give the model frontmatter containing a comment line, so there is
    // something to reflow without touching a parsed metadata key.
    std::fs::write(
        repo.path().join("models/m.sql"),
        "---\nunique_key: [customer_id]\n#  reflow comment\n---\nSELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id\n",
    )
    .expect("write frontmatter");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "add frontmatter comment");

    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    // A true formatting-only reflow: swap one interior double-space for a
    // space+tab inside the comment — same byte length, so
    // `strip_frontmatter`'s length-preserving blank-out produces
    // byte-identical stripped SQL, and the comment carries no parsed
    // metadata key, so `ModelMetadata` is also unchanged. This is not the
    // no-op "write the same bytes back" case Phase 4's original test used —
    // the file's bytes genuinely differ here.
    let original = std::fs::read_to_string(repo.path().join("models/m.sql")).expect("read");
    assert!(
        original.contains("#  reflow comment"),
        "fixture sanity: expected the double-space comment to still be present"
    );
    let reflowed = original.replacen("#  reflow comment", "# \treflow comment", 1);
    assert_eq!(
        original.len(),
        reflowed.len(),
        "the reflow must be byte-length-preserving for this test to be meaningful"
    );
    assert_ne!(original, reflowed, "the reflow must actually change bytes");
    std::fs::write(repo.path().join("models/m.sql"), &reflowed).expect("rewrite");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(
        !set.names.contains("m"),
        "an unchanged file must not be in the edited set: {:?}",
        set.names
    );
}

#[test]
fn edited_set_flags_a_frontmatter_only_edit() {
    let _guard = lock();
    let repo = fixture_repo();
    // Give the model frontmatter so we have something to edit in it.
    std::fs::write(
        repo.path().join("models/m.sql"),
        "-- unique_key: [customer_id]\nSELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id\n",
    )
    .expect("write");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "add frontmatter");

    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(
        repo.path().join("models/m.sql"),
        "-- unique_key: [order_id]\nSELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id\n",
    )
    .expect("edit frontmatter only");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "edit frontmatter");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(
        set.names.contains("m"),
        "a frontmatter-only edit must be in the edited set (Δ2): {:?}",
        set.names
    );
}

#[test]
fn edited_set_flags_a_smelt_yml_model_override() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(
        repo.path().join("smelt.yml"),
        "name: fixture\nmodels:\n  m:\n    materialization: table\n",
    )
    .expect("edit smelt.yml");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(set.names.contains("m"), "names={:?}", set.names);
    assert!(!set.project_config_changed);
}

#[test]
fn edited_set_flags_a_project_level_config_change() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(
        repo.path().join("smelt.yml"),
        "name: fixture\ndefault_materialization: table\n",
    )
    .expect("edit smelt.yml");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(
        set.names.is_empty(),
        "no model override touched, so no model should be edited: {:?}",
        set.names
    );
    assert!(set.project_config_changed);
}

#[test]
fn edited_set_flags_a_changed_source_declaration() {
    let _guard = lock();
    let repo = fixture_repo();
    std::fs::create_dir_all(repo.path().join("models/sources")).expect("mkdir");
    std::fs::write(
        repo.path().join("models/sources/orders.yml"),
        "columns:\n  - name: id\n    type: bigint\n",
    )
    .expect("write source");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "add source");

    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(
        repo.path().join("models/sources/orders.yml"),
        "columns:\n  - name: id\n    type: bigint\n  - name: amount\n    type: double\n",
    )
    .expect("edit source");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(
        set.names.contains("orders"),
        "the bare dotted source name (leading `sources` stripped) must be edited: {:?}",
        set.names
    );
}

#[test]
fn edited_set_flags_one_sided_files() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(repo.path().join("models/new_model.sql"), "SELECT 1\n")
        .expect("write new model");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(
        set.names.contains("new_model"),
        "a model present only in the working tree must be edited: {:?}",
        set.names
    );
}
