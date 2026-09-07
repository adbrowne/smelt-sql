use std::fs;
use std::path::{Path, PathBuf};

use super::test_only_files;

/// The admission/proof directories this gate covers, relative to the repo
/// root. Every `*.rs` file under these directories (recursively — mirrors
/// `crates/smelt-core/tests/hardening_budget.rs`'s `count_println_in_src_dir`
/// idiom) is scanned; a new file dropped into either directory is picked up
/// automatically rather than needing to be added to a hardcoded list.
const SCANNED_DIRS: &[&str] = &[
    "crates/smelt-logical/src/analysis",
    "crates/smelt-logical/src/rules",
    "crates/smelt-logical/src/maintenance",
    "crates/smelt-logical/src/backbuild",
];

/// Files under `SCANNED_DIRS` excluded from the gate, each with a reason.
/// Empty as of the cumulative-classifier migration (`docs/outcomes/
/// 20260904-walk-migration-residue/phases/04-plan.md`) — `rules/cumulative.rs`
/// was the last entry. A new entry here is a live, reviewed exception to the
/// property composition walk invariant (`docs/specs/architecture.md`
/// §"Property composition walk rule"), not a place to silently park a fresh
/// whole-SQL scan; it requires a reviewer sign-off note in the commit that
/// adds it.
const KNOWN_NONCOMPLIANT: &[&str] = &[];

/// Recursively collect every `*.rs` file under `dir` (relative to `root`),
/// mirroring `hardening_budget.rs`'s `count_println_in_src_dir` traversal.
fn collect_rs_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(root, &path, out);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            out.push(
                path.strip_prefix(root)
                    .expect("scanned path is under repo root")
                    .to_path_buf(),
            );
        }
    }
}

/// The scan set: every `*.rs` file under `SCANNED_DIRS`, minus
/// `KNOWN_NONCOMPLIANT`, as repo-root-relative paths (sorted for
/// deterministic diagnostics).
pub(crate) fn scanned_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in SCANNED_DIRS {
        collect_rs_files(root, &root.join(dir), &mut files);
    }
    files.retain(|rel| {
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        !KNOWN_NONCOMPLIANT.contains(&rel_str.as_str())
    });
    files.retain(|rel| !test_only_files::is_test_only(root, rel));
    files.sort();
    files
}
