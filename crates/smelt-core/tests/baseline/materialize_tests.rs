use smelt_core::baseline::{
    edited_set, materialize, materialize_in, resolve_baseline, BaselineError,
};
use smelt_core::sources::discover_source_infos;
use smelt_core::workspace::load_workspace;

use crate::fixtures::{fixture_repo, fixture_repo_wide, git, git_commit, git_query, lock};

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

    // A private scratch parent, not `std::env::temp_dir()`: any concurrent
    // `materialize` call elsewhere in this binary (or another test binary
    // running in parallel) creates and drops its own `smelt-baseline-*`
    // entries in the shared system temp dir, so a hygiene assertion there
    // races everything else on the box.
    let scratch_parent = tempfile::tempdir().expect("scratch parent");

    let err = materialize_in(&resolved, scratch_parent.path())
        .expect_err("bogus commit must fail materialize");
    assert!(matches!(err, BaselineError::Archive { .. }), "{err:?}");

    let leftover: Vec<_> = std::fs::read_dir(scratch_parent.path())
        .expect("read scratch parent")
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        leftover.is_empty(),
        "no scratch entry may survive a failed materialize, found: {leftover:?}"
    );
}

#[test]
fn checkout_scratch_is_deleted_on_drop_uses_the_given_parent() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let scratch_parent = tempfile::tempdir().expect("scratch parent");

    let checkout = materialize_in(&resolved, scratch_parent.path()).expect("materialize_in");
    assert!(
        checkout.project_root().starts_with(scratch_parent.path()),
        "materialize_in must put its scratch under the supplied parent"
    );
    drop(checkout);

    let leftover: Vec<_> = std::fs::read_dir(scratch_parent.path())
        .expect("read scratch parent")
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        leftover.is_empty(),
        "scratch parent must be empty again after the checkout drops, found: {leftover:?}"
    );
}

#[test]
fn materialize_defaults_its_scratch_parent_to_the_system_temp_dir() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");

    let checkout = materialize(&resolved).expect("materialize");
    let canonical_temp_dir =
        std::fs::canonicalize(std::env::temp_dir()).unwrap_or_else(|_| std::env::temp_dir());
    let canonical_root = std::fs::canonicalize(checkout.project_root())
        .unwrap_or_else(|_| checkout.project_root().to_path_buf());
    assert!(
        canonical_root.starts_with(&canonical_temp_dir),
        "plain materialize must default its scratch parent to std::env::temp_dir(), got {canonical_root:?} not under {canonical_temp_dir:?}"
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
///
/// Scans every file under `src/baseline/` (the module became a directory
/// with the git-subcommand surface, `git.rs`, split out from the pure
/// workspace-comparison half, `edited_set.rs`) rather than a single path,
/// so the guard keeps covering the whole module as it grows.
#[test]
fn git_surface_uses_no_mutating_subcommand() {
    let _guard = lock();
    let banned = [
        "\"checkout\"",
        "\"worktree\"",
        "\"stash\"",
        "\"read-tree\"",
        "\"update-ref\"",
        "\"commit\"",
    ];
    for (path, src) in [
        ("mod.rs", include_str!("../../src/baseline/mod.rs")),
        ("git.rs", include_str!("../../src/baseline/git.rs")),
        (
            "edited_set.rs",
            include_str!("../../src/baseline/edited_set.rs"),
        ),
    ] {
        for pattern in banned {
            assert!(
                !src.contains(pattern),
                "src/baseline/{path} must not use the mutating git subcommand {pattern}"
            );
        }
    }
}
