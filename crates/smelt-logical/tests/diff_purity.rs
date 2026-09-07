//! Structural gate for `docs/specs/property_diff.md` §Constraints item 2
//! ("Diff purity"): `smelt_logical::analysis::diff` (`diff_profiles` and its
//! helpers) performs no I/O and reads no ledger, snapshot, or backend. A
//! later phase (baseline materialisation via `git archive`) must never
//! import anything git-, filesystem-, process-, or backend-shaped into this
//! module — that seam belongs one layer up, in the caller that builds
//! [`smelt_logical::analysis::diff::DiffGraph`].
//!
//! Mechanism: scan the module's source text for a short list of tokens that
//! would only appear if the purity boundary were crossed. This is
//! deliberately blunt (a source-text grep, not a dependency-graph
//! assertion) because the module has no external I/O dependency to assert
//! *absence* of — the danger is a future edit adding one inline
//! (`std::fs`, `std::process::Command`) or importing a crate that performs
//! it (`smelt_state`, `smelt_backend`), mirroring the `walk_coverage.rs`
//! idiom of a source-text scan for a structural rule.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/smelt-logical has a parent dir")
        .parent()
        .expect("crates/ has a parent dir")
        .to_path_buf()
}

/// Tokens that would only appear if `diff.rs` crossed the purity boundary:
/// direct I/O (`std::fs`, `std::process`, `Command`) or a dependency on a
/// crate that performs it (`smelt_state`, `smelt_backend`) or on git itself.
const FORBIDDEN_TOKENS: &[&str] = &[
    "std::fs",
    "std::process",
    "Command::new",
    "smelt_state",
    "smelt_backend",
    "git_archive",
    "git2::",
    "Repository::open",
];

/// `diff.rs` split into `diff/{mod,change_kind,change_types,diff_graph,
/// diff_property_set,diff_cell_verdicts,diff_profile,tests}.rs` — every file
/// under the directory is part of the module this gate covers, so the scan
/// walks the whole directory rather than a single path.
fn diff_module_files() -> Vec<PathBuf> {
    let dir = repo_root().join("crates/smelt-logical/src/analysis/diff");
    let mut out = Vec::new();
    let entries = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("diff_purity gate could not read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("diff_purity gate could not read a dir entry: {e}"))
            .path();
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    assert!(
        !out.is_empty(),
        "diff_purity gate found no .rs files under {}",
        dir.display()
    );
    out
}

#[test]
fn diff_module_performs_no_io_reads_no_ledger_snapshot_or_backend() {
    let mut violations = Vec::new();
    for path in diff_module_files() {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("diff_purity gate could not read {}: {e}", path.display()));
        for token in FORBIDDEN_TOKENS {
            if text.contains(token) {
                violations.push(format!("{} in {}", token, path.display()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "diff_profiles must stay a pure function over two profile maps and a graph \
         (docs/specs/property_diff.md §Constraints item 2, \"Diff purity\") — found \
         forbidden token(s) {violations:?}. A baseline-materialisation seam (git \
         archive, workspace loading) belongs in the caller that builds DiffGraph, not here.",
    );
}
