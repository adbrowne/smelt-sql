//! Structural gate for `docs/specs/model_properties.md` §"Skeleton-source
//! closure" / criterion 3 of `docs/outcomes/20260904-walk-migration-residue/
//! outcome.md`: every `JoinContext::new()` call site in `smelt-logical`'s
//! production admission/proof surface (`src/maintenance/`, `src/analysis/`)
//! must be either a documented context *builder* (a function whose whole job
//! is constructing a `JoinContext` from some caller-supplied facts, which the
//! call site then immediately populates) or a documented no-op (the call
//! site reads no context-dependent field of whatever it feeds the empty
//! context into). Both are tagged inline with a trailing or immediately
//! preceding `// join-context: <reason>` comment — this gate requires the
//! tag, not any particular wording, so a route that starts silently rebuilding
//! an empty context (rather than reusing a caller-supplied one) is caught the
//! moment it lands untagged, not left to be noticed later.
//!
//! Mechanism mirrors `walk_coverage.rs`: scan each target file's *production*
//! text (every `#[cfg(test)]`-annotated item's span excluded) for the literal
//! `JoinContext::new()`, and require the tag on the same line or the line
//! immediately before. Whole files declared under `#[cfg(test)]` in their
//! parent module (see `support/test_only_files.rs`) are excluded from the
//! scanned set entirely — a `#[cfg(test)] mod tests { .. }` *block* split out
//! into its own file is still test-only even though nothing inside the file
//! itself carries the attribute.

#[path = "../support/test_only_files.rs"]
mod test_only_files;

mod classify;
mod scan;

use std::path::PathBuf;

use classify::unclassified_sites;
use scan::scanned_files;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/smelt-logical has a parent dir")
        .parent()
        .expect("crates/ has a parent dir")
        .to_path_buf()
}

/// Regression lock for the test-file blind spot: `analysis/walk/tests.rs` is
/// declared `#[cfg(test)] mod tests;` in `analysis/walk/mod.rs`, so it must be
/// excluded from the scanned set, while `analysis/walk/mod.rs` itself (plain
/// production module) must still be included.
#[test]
fn gate_scans_production_walk_sources() {
    let root = repo_root();
    let files = scanned_files(&root);
    let walk_tests = PathBuf::from("crates/smelt-logical/src/analysis/walk/tests.rs");
    let walk_mod = PathBuf::from("crates/smelt-logical/src/analysis/walk/mod.rs");
    assert!(
        !files.contains(&walk_tests),
        "expected {} to be excluded as test-only, got it in the scanned set",
        walk_tests.display()
    );
    assert!(
        files.contains(&walk_mod),
        "expected {} to remain in the scanned set",
        walk_mod.display()
    );
}

#[test]
fn every_production_join_context_new_is_tagged() {
    let root = repo_root();
    let mut failures = Vec::new();
    for rel in scanned_files(&root) {
        let abs = root.join(&rel);
        for line in unclassified_sites(&abs) {
            failures.push(format!("{}:{line}", rel.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "every `JoinContext::new()` call site in smelt-logical's production admission/proof \
         surface must carry a `// join-context: <reason>` tag (same line or the line \
         immediately before) classifying it as a builder or a documented no-context-field \
         no-op — untagged site(s):\n{}",
        failures.join("\n")
    );
}
