//! Constraint 8 ("no repository mutation") guards: a real materialize
//! round-trip must leave the repository byte-unchanged, and the module's
//! own source must contain no mutating git subcommand literal.

use smelt_core::baseline::{edited_set, materialize, resolve_baseline};
use smelt_core::sources::discover_source_infos;
use smelt_core::workspace::load_workspace;

use crate::fixtures::{fixture_repo, git, git_commit, git_query, lock};

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
/// Walks every `.rs` file under `src/baseline/` (recursing into
/// subdirectories such as `git/`, the split-out git-subcommand surface)
/// rather than an enumerated file list, so the guard keeps covering the
/// whole module as it grows without needing a matching edit here.
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
    let baseline_src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/baseline");
    for path in rust_files_under(&baseline_src_dir) {
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for pattern in banned {
            assert!(
                !src.contains(pattern),
                "{} must not use the mutating git subcommand {pattern}",
                path.display()
            );
        }
    }
}

/// Recursively collect every `.rs` file under `dir`.
fn rust_files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files_under(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files
}
