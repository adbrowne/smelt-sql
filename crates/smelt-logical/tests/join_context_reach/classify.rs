use std::fs;
use std::path::Path;

const TAG: &str = "join-context:";

/// How many lines directly above a call site's own line this gate will scan
/// for its `join-context:` tag — wide enough to cover a multi-line `//`
/// comment block sitting immediately above the call (this crate's own
/// convention, e.g. a three-line "builder (...)" explanation), but bounded
/// so a tag genuinely unrelated to this call site (attached to a different,
/// earlier statement) is never mistaken for this one's.
const LOOKBACK_LINES: usize = 6;

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

/// Every 1-based line number in `path` where `JoinContext::new()` appears in
/// actual production code (outside any `#[cfg(test)]` span, and not merely
/// mentioned inside a `//`/`///` comment) with no `join-context:` tag on the
/// same line or within the contiguous `//`-comment block directly above it.
pub(crate) fn unclassified_sites(path: &Path) -> Vec<usize> {
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
