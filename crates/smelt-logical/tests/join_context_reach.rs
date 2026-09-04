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
//! immediately before.

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

const SCANNED_DIRS: &[&str] = &[
    "crates/smelt-logical/src/analysis",
    "crates/smelt-logical/src/maintenance",
];

const TAG: &str = "join-context:";

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

fn scanned_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in SCANNED_DIRS {
        collect_rs_files(root, &root.join(dir), &mut files);
    }
    files.sort();
    files
}

/// Line-index `(start, end)` spans (inclusive, 0-based) of every
/// `#[cfg(test)]`-annotated item in `lines` — same brace-counting idiom as
/// `walk_coverage.rs`'s `cfg_test_spans`, duplicated locally rather than
/// shared across integration test binaries (each `tests/*.rs` file compiles
/// as its own crate).
fn cfg_test_spans(lines: &[String]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].trim_start().starts_with("#[cfg(test)]") {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0i32;
        let mut opened = false;
        let mut end = start;
        let mut j = start;
        while j < lines.len() {
            let line = &lines[j];
            for ch in line.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        opened = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            end = j;
            if opened && depth <= 0 {
                break;
            }
            if !opened && line.trim_end().ends_with(';') {
                break;
            }
            j += 1;
        }
        spans.push((start, end));
        i = end + 1;
    }
    spans
}

fn is_within_any_span(spans: &[(usize, usize)], i: usize) -> bool {
    spans.iter().any(|(start, end)| i >= *start && i <= *end)
}

/// How many lines directly above a call site's own line this gate will scan
/// for its `join-context:` tag — wide enough to cover a multi-line `//`
/// comment block sitting immediately above the call (this crate's own
/// convention, e.g. a three-line "builder (...)" explanation), but bounded
/// so a tag genuinely unrelated to this call site (attached to a different,
/// earlier statement) is never mistaken for this one's.
const LOOKBACK_LINES: usize = 6;

/// Every 1-based line number in `path` where `JoinContext::new()` appears in
/// actual production code (outside any `#[cfg(test)]` span, and not merely
/// mentioned inside a `//`/`///` comment) with no `join-context:` tag on the
/// same line or within the contiguous `//`-comment block directly above it.
fn unclassified_sites(path: &Path) -> Vec<usize> {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let test_spans = cfg_test_spans(&lines);

    let mut violations = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if is_within_any_span(&test_spans, i) {
            continue;
        }
        if line.trim_start().starts_with("//") {
            // A comment merely mentioning `JoinContext::new()` in prose
            // (e.g. `affected_keys.rs`'s own doc comment) is not a call
            // site.
            continue;
        }
        if !line.contains("JoinContext::new()") {
            continue;
        }
        if line.contains(TAG) {
            continue;
        }
        let tagged_above = (1..=LOOKBACK_LINES).any(|back| {
            i >= back
                && lines[i - back].trim_start().starts_with("//")
                && lines[i - back].contains(TAG)
        });
        if !tagged_above {
            violations.push(i + 1);
        }
    }
    violations
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
