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

#[path = "support/test_only_files.rs"]
mod test_only_files;

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
fn scanned_files(root: &Path) -> Vec<PathBuf> {
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

const TAGS: &[&str] = &["leaf classifier", "advisory heuristic"];

/// Every identifier in `lines` bound (via `let`/`let mut`) to an expression
/// containing `.to_uppercase()` or `.to_lowercase()` — the case-folded
/// free-text buffer a scan like the pre-migration `cumulative.rs`'s
/// `upper_sql.contains(&pattern)` reads from. `is_raw_scan_line` uses this to
/// catch the non-literal scan form a bare `.contains("` grep cannot see: the
/// pattern argument is a variable, not a string literal, but the receiver is
/// still free-text scanned over a whole case-folded buffer.
fn case_folded_variables(lines: &[String]) -> std::collections::HashSet<String> {
    let mut vars = std::collections::HashSet::new();
    for line in lines {
        let Some(eq_pos) = line.find('=') else {
            continue;
        };
        let (lhs, rhs) = line.split_at(eq_pos);
        if !(rhs.contains(".to_uppercase()") || rhs.contains(".to_lowercase()")) {
            continue;
        }
        let ident_part = lhs
            .trim()
            .strip_prefix("let mut ")
            .or_else(|| lhs.trim().strip_prefix("let "))
            .unwrap_or(lhs.trim());
        let ident: String = ident_part
            .trim()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !ident.is_empty() {
            vars.insert(ident);
        }
    }
    vars
}

/// The identifier immediately before a `.contains(` call on `line`, if any
/// (e.g. `upper_sql` in `upper_sql.contains(&pattern)`).
fn contains_receiver(line: &str) -> Option<String> {
    let idx = line.find(".contains(")?;
    let ident: String = line[..idx]
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let ident: String = ident.chars().rev().collect();
    (!ident.is_empty()).then_some(ident)
}

/// A raw substring text-scan: `.contains("` with a string-literal argument,
/// or `<ident>.contains(...)` where `<ident>` is bound elsewhere in the same
/// production source to a `.to_uppercase()`/`.to_lowercase()` expression
/// ([`case_folded_variables`]) — the case-folded-variable scan form (e.g.
/// `let upper_sql = sql.to_uppercase(); … upper_sql.contains(&pattern)`) a
/// literal-only grep cannot see. This is the pattern the spec restricts to
/// leaf classifiers/advisory heuristics — it excludes exact-match identifier
/// lookups (`SqlFunction::from_name(&name.to_uppercase())`, `a.to_lowercase()
/// == b.to_lowercase()`) and ordinary collection-membership `.contains(&x)`
/// on a receiver that isn't a case-folded buffer, which are benign, not
/// keyword-in-free-text scanning.
fn is_raw_scan_line(line: &str, folded_vars: &std::collections::HashSet<String>) -> bool {
    if line.contains(".contains(\"") {
        return true;
    }
    contains_receiver(line).is_some_and(|ident| folded_vars.contains(&ident))
}

fn is_fn_signature(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("pub(crate) fn ")
}

/// Production lines — every line in the file *except* those inside a
/// `#[cfg(test)]`-annotated item's own span — plus whether the
/// module-level `//!` doc block carries a classification tag.
///
/// Deliberately **not** "everything strictly before the first
/// `#[cfg(test)]` line": that truncation assumption is false for at least
/// one file in this crate (`maintenance/propagate.rs` has a
/// `#[cfg(test)] mod day_interval_tests { .. }` block at line 85, followed
/// by ~450 lines of production code — `normalize` and friends — followed
/// by two more test modules). A file may interleave test modules and
/// production code any number of times; each `#[cfg(test)]` span is
/// excluded individually via [`cfg_test_spans`], and everything else is
/// scanned, regardless of how many test blocks precede it.
struct ProductionSource {
    lines: Vec<String>,
    module_doc_is_classified: bool,
}

/// Line-index `(start, end)` spans (inclusive, 0-based) of every
/// `#[cfg(test)]`-annotated item in `lines`: the attribute line itself
/// through the closing brace of the item it annotates, tracked by brace
/// depth from that item's own first `{`. Mirrors [`function_spans`]'s
/// brace-counting idiom, applied to attribute-marked items instead of `fn`
/// signatures. A same-line-terminated item with no braces at all (e.g. a
/// bare `#[cfg(test)] mod tests;` declaration) closes at its own
/// semicolon — not used by this crate's style today, but handled so an
/// unbalanced file fails loud (an unterminated span) rather than silently
/// swallowing the rest of the file.
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
                // A brace-less item (e.g. `mod tests;`) — ends here.
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

/// Blanks every line inside a `#[cfg(test)]` span to an empty string
/// rather than dropping it, so `lines[i]` still corresponds to the
/// original file's 1-based line `i + 1` — `unclassified_raw_scans` reports
/// that number directly, and a shifted index would point a violation at
/// the wrong line the moment a file has more than one test span.
fn load_production_source(path: &Path) -> ProductionSource {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let all_lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let test_spans = cfg_test_spans(&all_lines);

    let mut lines = Vec::with_capacity(all_lines.len());
    let mut module_doc_is_classified = false;
    for (i, line) in all_lines.iter().enumerate() {
        if is_within_any_span(&test_spans, i) {
            lines.push(String::new());
            continue;
        }
        if line.trim_start().starts_with("//!") {
            let lower = line.to_lowercase();
            if TAGS.iter().any(|t| lower.contains(t)) {
                module_doc_is_classified = true;
            }
        }
        lines.push(line.clone());
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
    let folded_vars = case_folded_variables(&source.lines);
    let mut violations = Vec::new();

    for (i, line) in source.lines.iter().enumerate() {
        if !is_raw_scan_line(line, &folded_vars) {
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

/// Regression lock for the test-file blind spot: `maintenance/choice/tests.rs`
/// and `maintenance/choice/write_suppression_tests.rs` are both declared
/// `#[cfg(test)] mod <stem>;` in `maintenance/choice/mod.rs`, so both must be
/// excluded from the scanned set, while `maintenance/choice/mod.rs` itself
/// must still be included.
#[test]
fn gate_scans_production_choice_sources() {
    let root = repo_root();
    let files = scanned_files(&root);
    let choice_tests = PathBuf::from("crates/smelt-logical/src/maintenance/choice/tests.rs");
    let write_suppression_tests =
        PathBuf::from("crates/smelt-logical/src/maintenance/choice/write_suppression_tests.rs");
    let choice_mod = PathBuf::from("crates/smelt-logical/src/maintenance/choice/mod.rs");
    assert!(
        !files.contains(&choice_tests),
        "expected {} to be excluded as test-only, got it in the scanned set",
        choice_tests.display()
    );
    assert!(
        !files.contains(&write_suppression_tests),
        "expected {} to be excluded as test-only, got it in the scanned set",
        write_suppression_tests.display()
    );
    assert!(
        files.contains(&choice_mod),
        "expected {} to remain in the scanned set",
        choice_mod.display()
    );
}

/// The committed tree carries no unclassified raw text-scan in the
/// admission/proof surface.
#[test]
fn admission_paths_have_no_raw_text_scans() {
    let root = repo_root();
    let files = scanned_files(&root);
    assert!(
        !files.is_empty(),
        "expected at least one *.rs file under {SCANNED_DIRS:?}"
    );
    let mut all_violations = Vec::new();
    for rel in &files {
        let path = root.join(rel);
        assert!(path.exists(), "{} not found under {root:?}", rel.display());
        for (line_no, text) in unclassified_raw_scans(&path) {
            all_violations.push(format!("{}:{line_no}: {text}", rel.display()));
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

/// Regression lock for the "first-`#[cfg(test)]`-truncates-the-rest" blind
/// spot (review finding on `docs/plans/20260808-substrate-unification.md`
/// Phase 6): `maintenance/propagate.rs`'s real shape is a
/// `#[cfg(test)] mod day_interval_tests { .. }` block, then ~450 lines of
/// production code (`normalize` and friends), then two more test modules.
/// `load_production_source` must scan the production span that sits
/// *between* two test spans, not just "everything before the first
/// `#[cfg(test)]` line".
#[test]
fn load_production_source_scans_production_code_interleaved_between_cfg_test_blocks() {
    let dir = std::env::temp_dir().join(format!(
        "smelt-walk-coverage-probe-interleaved-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp probe dir");
    let probe = dir.join("propagate_shape.rs");
    fs::write(
        &probe,
        "#[cfg(test)]\n\
         mod day_interval_tests {\n\
         \x20   #[test]\n\
         \x20   fn probe() {\n\
         \x20       assert!(true);\n\
         \x20   }\n\
         }\n\
         \n\
         fn normalize(x: &str) -> bool {\n\
         \x20   x.to_lowercase().contains(\"group by\")\n\
         }\n\
         \n\
         #[cfg(test)]\n\
         mod later_tests {\n\
         \x20   #[test]\n\
         \x20   fn probe2() {\n\
         \x20       assert!(true);\n\
         \x20   }\n\
         }\n",
    )
    .expect("write probe fixture");

    let source = load_production_source(&probe);
    let joined = source.lines.join("\n");

    assert!(
        joined.contains("fn normalize"),
        "expected the production fn sandwiched between two cfg(test) blocks to be \
         scanned, got:\n{joined}"
    );
    assert!(
        joined.contains("x.to_lowercase().contains(\"group by\")"),
        "expected the production scan site itself to survive exclusion, got:\n{joined}"
    );
    assert!(
        !joined.contains("day_interval_tests"),
        "expected the first cfg(test) block's own content to be excluded, got:\n{joined}"
    );
    assert!(
        !joined.contains("later_tests"),
        "expected the second cfg(test) block's own content to be excluded, got:\n{joined}"
    );
    assert!(
        !joined.contains("assert!(true)"),
        "expected every test-module body line to be excluded, got:\n{joined}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// End-to-end twin of the span test above: the full gate
/// (`unclassified_raw_scans`) must actually flag the unclassified scan
/// sitting in the interleaved production span, and must not flag anything
/// inside either surrounding test module.
#[test]
fn gate_detects_a_violation_interleaved_between_two_cfg_test_blocks() {
    let dir = std::env::temp_dir().join(format!(
        "smelt-walk-coverage-probe-interleaved-gate-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp probe dir");
    let probe = dir.join("propagate_shape_gate.rs");
    fs::write(
        &probe,
        "#[cfg(test)]\n\
         mod day_interval_tests {\n\
         \x20   #[test]\n\
         \x20   fn probe() {\n\
         \x20       assert!(\"y\".to_lowercase().contains(\"should not be flagged\"));\n\
         \x20   }\n\
         }\n\
         \n\
         fn normalize(x: &str) -> bool {\n\
         \x20   x.to_lowercase().contains(\"group by\")\n\
         }\n\
         \n\
         #[cfg(test)]\n\
         mod later_tests {\n\
         \x20   #[test]\n\
         \x20   fn probe2() {\n\
         \x20       assert!(\"z\".to_lowercase().contains(\"also not flagged\"));\n\
         \x20   }\n\
         }\n",
    )
    .expect("write probe fixture");

    let violations = unclassified_raw_scans(&probe);
    assert_eq!(
        violations.len(),
        1,
        "expected exactly one violation (the interleaved production scan), got: {violations:?}"
    );
    assert!(
        violations[0]
            .1
            .contains("x.to_lowercase().contains(\"group by\")"),
        "expected the flagged line to be the production scan, got: {violations:?}"
    );
    // The flagged line number must point at the actual production line in
    // the file, not a shifted index from dropping the preceding test span.
    let expected_line_no = std::fs::read_to_string(&probe)
        .expect("reread probe")
        .lines()
        .position(|l| l.contains("x.to_lowercase().contains(\"group by\")"))
        .map(|idx| idx + 1)
        .expect("scan line present in probe");
    assert_eq!(violations[0].0, expected_line_no);

    let _ = fs::remove_dir_all(&dir);
}

/// `rules/cumulative.rs` is no longer parked in `KNOWN_NONCOMPLIANT` — the
/// keyed-admission whole-SQL scans it carried (`docs/outcomes/
/// 20260904-walk-migration-residue/phases/04-plan.md`) migrated onto the
/// walk. Regression lock: a future re-addition of the skip-list entry (e.g.
/// to hide a reintroduced scan) makes this fail.
#[test]
fn cumulative_rs_is_covered_by_the_gate() {
    let root = repo_root();
    let files = scanned_files(&root);
    let rel = PathBuf::from("crates/smelt-logical/src/rules/cumulative.rs");
    assert!(
        files.contains(&rel),
        "expected {} to be scanned by the gate (not skip-listed), got: {files:?}",
        rel.display()
    );
}

/// The widened `is_raw_scan_line` catches the non-literal, case-folded-
/// variable scan form (`let upper = s.to_uppercase(); … upper.contains(&x)`)
/// a bare `.contains("` grep cannot see — and does not flag an ordinary
/// collection-membership `.contains(&x)` on a receiver that was never
/// case-folded.
#[test]
fn detects_an_unclassified_case_folded_variable_scan() {
    let dir = std::env::temp_dir().join(format!(
        "smelt-walk-coverage-probe-case-folded-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp probe dir");
    let probe = dir.join("case_folded_probe.rs");
    fs::write(
        &probe,
        "fn scans_case_folded(s: &str, pattern: &str) -> bool {\n\
         \x20   let upper = s.to_uppercase();\n\
         \x20   upper.contains(pattern)\n\
         }\n\n\
         fn checks_membership(names: &Vec<String>, name: &str) -> bool {\n\
         \x20   names.contains(&name.to_string())\n\
         }\n",
    )
    .expect("write probe fixture");

    let violations = unclassified_raw_scans(&probe);
    assert!(
        violations
            .iter()
            .any(|(_, text)| text.contains("upper.contains(pattern)")),
        "expected the case-folded-variable scan to be flagged, got: {violations:?}"
    );
    assert!(
        !violations
            .iter()
            .any(|(_, text)| text.contains("names.contains(&name.to_string())")),
        "expected the unrelated collection-membership check to survive, got: {violations:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Claims `docs/specs/model_properties.md` §Known Divergences must not restate once the
/// composition walk actually closes them (`docs/outcomes/20260904-walk-migration-residue/
/// outcome.md` phase 7). Each entry is the exact substring the closed claim used; a stale
/// restatement of either fails this test.
const CLOSED_WALK_GAP_CLAIMS: &[&str] = &[
    "Only one maintenance-cell route consults a declared-RI closure today",
    "whole-SQL `OVER(` scan",
];

/// Extract the `## Known Divergences` section body (up to the next `## ` heading) from a
/// `model_properties.md`-shaped markdown document. `None` if the heading is missing, so a
/// caller can distinguish "found the section, it's clean" from "never looked".
fn known_divergences_section(markdown: &str) -> Option<&str> {
    let start = markdown.find("## Known Divergences")?;
    let after_heading = &markdown[start..];
    let body_start = after_heading.find('\n')? + 1;
    let body = &after_heading[body_start..];
    let end = body.find("\n## ").unwrap_or(body.len());
    Some(&body[..end])
}

fn find_closed_walk_gap_claim(section: &str) -> Option<&'static str> {
    CLOSED_WALK_GAP_CLAIMS
        .iter()
        .copied()
        .find(|phrase| section.contains(phrase))
}

/// Durable regression lock for phase 7's divergence-bullet deletions: `model_properties.md`
/// §Known Divergences must never again claim only one maintenance-cell route consults a
/// declared-RI closure (closed by phase 5) or that a whole-SQL `OVER(` scan still governs
/// cumulative classification (closed by phase 4).
#[test]
fn spec_divergences_do_not_claim_closed_walk_gaps() {
    let root = repo_root();
    let spec_path = root.join("docs/specs/model_properties.md");
    let text = fs::read_to_string(&spec_path).expect("read docs/specs/model_properties.md");
    let section = known_divergences_section(&text)
        .expect("model_properties.md has a '## Known Divergences' heading");
    let found = find_closed_walk_gap_claim(section);
    assert!(
        found.is_none(),
        "docs/specs/model_properties.md §Known Divergences restates a claim the \
         20260904-walk-migration-residue outcome already closed: {found:?}"
    );
}

/// Guards the gate above against silently passing because it failed to locate the section at
/// all (e.g. a future heading rename) rather than because the section is actually clean.
#[test]
fn spec_divergence_gate_detects_a_stale_claim() {
    let synthetic = "# Spec\n\n\
         ## Known Divergences\n\n\
         - Only one maintenance-cell route consults a declared-RI closure today, in fact.\n\n\
         ## Future Extensions\n\nsomething unrelated\n";
    let section =
        known_divergences_section(synthetic).expect("synthetic doc has the expected heading");
    assert_eq!(
        find_closed_walk_gap_claim(section),
        Some("Only one maintenance-cell route consults a declared-RI closure today"),
        "gate failed to detect a stale claim planted in a synthetic §Known Divergences body"
    );
}
