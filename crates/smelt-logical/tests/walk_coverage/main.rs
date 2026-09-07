//! Structural gate for `docs/specs/architecture.md` §"Property composition
//! walk rule" / `docs/specs/model_properties.md` §Constraints "Composition
//! happens in the walk, not in scans": every raw substring text-scan
//! (`.contains("…")` on already-case-folded free text) in the admission/proof
//! surface of `smelt-logical` must be classified, in a doc comment, as either
//! a `Leaf classifier` (invoked by the shared composition walk over one
//! already-bounded node's own text) or an `Advisory heuristic` (a value that
//! never feeds a composition-relevant verdict). An unclassified new scan is
//! exactly the shape the invariant forbids — a substring check standing in
//! for the walk instead of being invoked by it — so this test fails on one.
//!
//! Mechanism (analogous to `crates/smelt-core/tests/hardening_budget.rs`):
//! read each target file's production text (everything before its single
//! trailing `#[cfg(test)] mod tests` block), find every `.contains("` call,
//! and require the classification tag either in the immediately preceding
//! `///` doc-comment block of the enclosing function, or in the file's
//! module-level `//!` doc block (a file-wide tag, used by `temporal.rs`,
//! whose `EffectiveWindow` walk is a deliberate whole-module divergence).
//! Whole files declared under `#[cfg(test)]` in their parent module (see
//! `support/test_only_files.rs`) are excluded from the scanned set entirely —
//! a `#[cfg(test)] mod tests { .. }` *block* split out into its own file
//! (e.g. `maintenance/choice/write_variant_tests.rs`) is still test-only even
//! though nothing inside the file itself carries the attribute.
//!
//! Split across sibling modules: `scan` (file collection), `classify` (the
//! raw-scan and `#[cfg(test)]`-span detection this gate runs), `gates` (the
//! `#[test]` functions exercising them), and `divergence` (a separate,
//! smaller gate on `docs/specs/model_properties.md` §Known Divergences that
//! lives here because it guards the same walk-migration invariant).

#[path = "../support/test_only_files.rs"]
mod test_only_files;

mod classify;
mod divergence;
mod gates;
mod scan;

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/smelt-logical has a parent dir")
        .parent()
        .expect("crates/ has a parent dir")
        .to_path_buf()
}
