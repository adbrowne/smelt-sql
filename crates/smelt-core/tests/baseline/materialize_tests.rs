use smelt_core::baseline::{materialize, resolve_baseline};

use crate::fixtures::{fixture_repo, fixture_repo_wide, git, git_commit, lock};

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
