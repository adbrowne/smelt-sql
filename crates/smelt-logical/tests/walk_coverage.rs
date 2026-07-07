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

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/smelt-logical has a parent dir")
        .parent()
        .expect("crates/ has a parent dir")
        .to_path_buf()
}

/// The admission/proof modules this gate covers. Deliberately excludes
/// `rules/cumulative.rs`: its cumulative-model admission gate predates this
/// plan's scope and is not yet migrated onto the shared walk or otherwise
/// classified — tracked as separate debt, not silently mislabeled here.
const SCANNED_FILES: &[&str] = &[
    "crates/smelt-logical/src/analysis/mod.rs",
    "crates/smelt-logical/src/analysis/source_bounds.rs",
    "crates/smelt-logical/src/analysis/temporal.rs",
    "crates/smelt-logical/src/analysis/monotonicity.rs",
    "crates/smelt-logical/src/analysis/functional_dependency.rs",
    "crates/smelt-logical/src/analysis/join_shape.rs",
    "crates/smelt-logical/src/analysis/bounded_domain.rs",
    "crates/smelt-logical/src/analysis/window_independence.rs",
    "crates/smelt-logical/src/analysis/walk.rs",
    "crates/smelt-logical/src/analysis/discriminants.rs",
    "crates/smelt-logical/src/analysis/presentation.rs",
    "crates/smelt-logical/src/rules/incremental.rs",
    "crates/smelt-logical/src/rules/rule_diagnostics.rs",
];

const TAGS: &[&str] = &["leaf classifier", "advisory heuristic"];

/// A raw substring text-scan: `.contains("` with a string-literal argument.
/// This is the pattern the spec restricts to leaf classifiers/advisory
/// heuristics — it excludes exact-match identifier lookups
/// (`SqlFunction::from_name(&name.to_uppercase())`, `a.to_lowercase() ==
/// b.to_lowercase()`), which are benign case-insensitive comparisons, not
/// keyword-in-free-text scanning.
fn is_raw_scan_line(line: &str) -> bool {
    line.contains(".contains(\"")
}

fn is_fn_signature(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("pub(crate) fn ")
}

/// Production lines (everything strictly before the file's `#[cfg(test)]`
/// line, or the whole file if absent) plus whether the module-level `//!`
/// doc block carries a classification tag.
struct ProductionSource {
    lines: Vec<String>,
    module_doc_is_classified: bool,
}

fn load_production_source(path: &Path) -> ProductionSource {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let mut lines = Vec::new();
    let mut module_doc_is_classified = false;
    for line in content.lines() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        if line.trim_start().starts_with("//!") {
            let lower = line.to_lowercase();
            if TAGS.iter().any(|t| lower.contains(t)) {
                module_doc_is_classified = true;
            }
        }
        lines.push(line.to_string());
    }
    ProductionSource {
        lines,
        module_doc_is_classified,
    }
}

/// Brace-counted `(start, end)` line-index span (inclusive) for every
/// top-level/impl-level `fn` signature in `lines`. Good enough for this
/// crate's style: no `fn` keyword appears in closures (`|x| { .. }`), so
/// brace-depth tracking from each signature line to its matching close is
/// unambiguous.
fn function_spans(lines: &[String]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for (start, line) in lines.iter().enumerate() {
        if !is_fn_signature(line) {
            continue;
        }
        let mut depth = 0i32;
        let mut opened = false;
        let mut end = start;
        for (i, l) in lines.iter().enumerate().skip(start) {
            for ch in l.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        opened = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            if opened && depth <= 0 {
                end = i;
                break;
            }
        }
        spans.push((start, end));
    }
    spans
}

/// Does the contiguous `///` doc-comment block immediately preceding
/// `fn_start` (skipping any `#[...]` attribute lines directly above the
/// signature) carry a classification tag?
fn doc_comment_is_classified(lines: &[String], fn_start: usize) -> bool {
    let mut i = fn_start;
    while i > 0 {
        let prev = lines[i - 1].trim_start();
        if prev.starts_with("#[") {
            i -= 1;
            continue;
        }
        if prev.starts_with("///") {
            let lower = prev.to_lowercase();
            if TAGS.iter().any(|t| lower.contains(t)) {
                return true;
            }
            i -= 1;
            continue;
        }
        break;
    }
    false
}

/// Every unclassified raw-scan site in `path`, as `(1-based line, trimmed
/// text)`.
fn unclassified_raw_scans(path: &Path) -> Vec<(usize, String)> {
    let source = load_production_source(path);
    if source.module_doc_is_classified {
        return Vec::new();
    }
    let spans = function_spans(&source.lines);
    let mut violations = Vec::new();

    for (i, line) in source.lines.iter().enumerate() {
        if !is_raw_scan_line(line) {
            continue;
        }
        // Innermost enclosing function: the span with the latest start that
        // still contains this line.
        let enclosing = spans
            .iter()
            .filter(|(start, end)| *start <= i && i <= *end)
            .max_by_key(|(start, _)| *start);

        let classified = match enclosing {
            Some((start, _)) => doc_comment_is_classified(&source.lines, *start),
            None => false,
        };
        if !classified {
            violations.push((i + 1, line.trim().to_string()));
        }
    }
    violations
}

/// The committed tree carries no unclassified raw text-scan in the
/// admission/proof surface.
#[test]
fn admission_paths_have_no_raw_text_scans() {
    let root = repo_root();
    let mut all_violations = Vec::new();
    for rel in SCANNED_FILES {
        let path = root.join(rel);
        assert!(path.exists(), "{rel} not found under {root:?}");
        for (line_no, text) in unclassified_raw_scans(&path) {
            all_violations.push(format!("{rel}:{line_no}: {text}"));
        }
    }
    assert!(
        all_violations.is_empty(),
        "unclassified raw text-scan(s) in the admission/proof surface — each \
         `.contains(\"…\")` on case-folded free text must be tagged `Leaf \
         classifier` or `Advisory heuristic` in the enclosing function's doc \
         comment (or the file's module-level `//!` doc), per \
         `docs/specs/architecture.md` §\"Property composition walk rule\":\n{}",
        all_violations.join("\n")
    );
}

/// Regression guard for the gate itself: an injected raw scan with no
/// classification tag, in a synthetic fixture file mirroring this crate's
/// style, is detected. Deterministic and self-contained — no network, no
/// dependence on the real source tree beyond `SCANNED_FILES` existing.
#[test]
fn detects_an_unclassified_injected_raw_scan() {
    let dir =
        std::env::temp_dir().join(format!("smelt-walk-coverage-probe-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp probe dir");
    let probe = dir.join("probe.rs");
    fs::write(
        &probe,
        "fn ok() -> bool {\n\
         \x20   // Leaf classifier for the test.\n\
         \x20   \"x\".to_uppercase().contains(\"UNBOUNDED\")\n\
         }\n\n\
         fn not_ok() -> bool {\n\
         \x20   let upper = \"x\".to_uppercase();\n\
         \x20   upper.contains(\"UNBOUNDED\")\n\
         }\n",
    )
    .expect("write probe fixture");

    // Note: `ok()`'s inline `//` comment is not a `///` doc comment, so it
    // does not classify `ok()` either — both functions should be flagged.
    // This asserts the *unclassified* half of that: `not_ok()` is caught.
    let violations = unclassified_raw_scans(&probe);
    assert!(
        violations
            .iter()
            .any(|(_, text)| text.contains("not_ok")
                || text.contains("upper.contains(\"UNBOUNDED\")")),
        "expected the injected raw scan in `not_ok` to be flagged, got: {violations:?}"
    );
    assert_eq!(
        violations.len(),
        2,
        "expected both unclassified scans (`//` is not `///`) to be flagged, got: {violations:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}
