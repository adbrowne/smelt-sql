use std::fs;
use std::path::Path;

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
pub(crate) struct ProductionSource {
    pub(crate) lines: Vec<String>,
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
pub(crate) fn load_production_source(path: &Path) -> ProductionSource {
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
pub(crate) fn unclassified_raw_scans(path: &Path) -> Vec<(usize, String)> {
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
