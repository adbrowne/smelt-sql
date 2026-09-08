//! Every production maintenance emitter that takes a `dialect` parameter must
//! actually dispatch on it — never hardcode `MaintenanceDialect::DuckDb`
//! regardless of what was passed in. Phases 1-3 of the `bigquery-correctness`
//! outcome fixed exactly this defect class (`emit_fingerprint_digest_select`,
//! `key_expr_for_columns`, `emit_repair_group_digest_select`); this is the
//! structural gate that keeps a new emitter from reintroducing it, since
//! criterion 3 was previously held only by per-emitter unit tests.
//!
//! A bare `MaintenanceDialect::DuckDb` is admissible only inside a `match`/
//! `matches!` dispatch on `dialect` (the line contains `=>` or
//! `matches!(dialect`) — never as a plain argument-position literal such as
//! `row_fingerprint_expr(cols, MaintenanceDialect::DuckDb)`. Occurrences
//! inside string literals (e.g. an assertion message that mentions the
//! variant by name) and inside `#[cfg(test)] mod ... { ... }` blocks are not
//! production hardcodes and are excluded.

use std::path::{Path, PathBuf};

fn maintenance_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src/maintenance"))
}

/// Every `.rs` file under `src/maintenance/`, recursively, read at test time
/// so a new file or submodule is covered the moment it exists.
fn maintenance_files() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("readable maintenance dir") {
            let path = entry.expect("readable dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut paths = Vec::new();
    walk(&maintenance_dir(), &mut paths);
    paths.sort();
    assert!(
        !paths.is_empty(),
        "no maintenance sources found — the gate would pass vacuously"
    );
    paths
        .into_iter()
        .map(|p| {
            let src = std::fs::read_to_string(&p).expect("readable source file");
            (p, src)
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CharKind {
    Code,
    Str,
    LineComment,
    BlockComment,
}

/// Tags every char of `source` with what lexical context it's in, so the
/// offender scan and the test-module stripper can both ignore quotes and
/// braces that appear inside a string or comment rather than in real code.
fn classify_chars(chars: &[char]) -> Vec<CharKind> {
    let n = chars.len();
    let mut kinds = vec![CharKind::Code; n];
    let mut state = CharKind::Code;
    let mut str_escape = false;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        match state {
            CharKind::Code => {
                if c == '"' {
                    state = CharKind::Str;
                    kinds[i] = CharKind::Str;
                } else if c == '/' && chars.get(i + 1) == Some(&'/') {
                    state = CharKind::LineComment;
                    kinds[i] = CharKind::LineComment;
                } else if c == '/' && chars.get(i + 1) == Some(&'*') {
                    state = CharKind::BlockComment;
                    kinds[i] = CharKind::BlockComment;
                } else {
                    kinds[i] = CharKind::Code;
                }
            }
            CharKind::Str => {
                kinds[i] = CharKind::Str;
                if str_escape {
                    str_escape = false;
                } else if c == '\\' {
                    str_escape = true;
                } else if c == '"' {
                    state = CharKind::Code;
                }
            }
            CharKind::LineComment => {
                kinds[i] = CharKind::LineComment;
                if c == '\n' {
                    state = CharKind::Code;
                }
            }
            CharKind::BlockComment => {
                kinds[i] = CharKind::BlockComment;
                if c == '*' && chars.get(i + 1) == Some(&'/') {
                    kinds[i + 1] = CharKind::BlockComment;
                    state = CharKind::Code;
                    i += 1;
                }
            }
        }
        i += 1;
    }
    kinds
}

fn matches_code_marker(chars: &[char], kinds: &[CharKind], pos: usize, marker: &[char]) -> bool {
    if pos + marker.len() > chars.len() {
        return false;
    }
    (0..marker.len())
        .all(|off| chars[pos + off] == marker[off] && kinds[pos + off] == CharKind::Code)
}

/// Drops every `#[cfg(test)]\nmod ... { ... }` block wholesale (brace-matched,
/// respecting strings/comments so a brace inside a test's SQL fixture string
/// doesn't miscount) — those are test fixtures, not production emission code.
fn strip_test_modules(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let kinds = classify_chars(&chars);
    let marker: Vec<char> = "#[cfg(test)]".chars().collect();
    let n = chars.len();
    let mut skip = vec![false; n];
    let mut i = 0;
    while i < n {
        if kinds[i] == CharKind::Code && matches_code_marker(&chars, &kinds, i, &marker) {
            let mut j = i;
            while j < n && !(chars[j] == '{' && kinds[j] == CharKind::Code) {
                j += 1;
            }
            if j < n {
                let mut depth = 1;
                let mut k = j + 1;
                while k < n && depth > 0 {
                    if kinds[k] == CharKind::Code {
                        if chars[k] == '{' {
                            depth += 1;
                        } else if chars[k] == '}' {
                            depth -= 1;
                        }
                    }
                    k += 1;
                }
                for slot in skip.iter_mut().take(k).skip(i) {
                    *slot = true;
                }
                i = k;
                continue;
            }
        }
        i += 1;
    }
    chars
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| !skip[*idx])
        .map(|(_, c)| c)
        .collect()
}

/// Every offending `MaintenanceDialect::DuckDb` occurrence, as `(1-based
/// line, trimmed line text)`, in `source` (which should already have test
/// modules stripped).
fn find_offenders(source: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = source.chars().collect();
    let kinds = classify_chars(&chars);
    let needle: Vec<char> = "MaintenanceDialect::DuckDb".chars().collect();
    let lines: Vec<&str> = source.lines().collect();

    let mut line_of = vec![0usize; chars.len()];
    let mut line_no = 0;
    for (idx, c) in chars.iter().enumerate() {
        line_of[idx] = line_no;
        if *c == '\n' {
            line_no += 1;
        }
    }

    let mut offenders = Vec::new();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        if chars[i..i + needle.len()] == needle[..] {
            if kinds[i] == CharKind::Code {
                let ln = line_of[i];
                let line_text = lines.get(ln).copied().unwrap_or("").trim().to_string();
                let dispatched = line_text.contains("=>") || line_text.contains("matches!(dialect");
                if !dispatched {
                    offenders.push((ln + 1, line_text));
                }
            }
            i += needle.len();
        } else {
            i += 1;
        }
    }
    offenders
}

#[test]
fn no_production_emitter_hardcodes_the_duckdb_dialect() {
    let mut all_offenders = Vec::new();
    for (path, src) in maintenance_files() {
        let stripped = strip_test_modules(&src);
        for (line, text) in find_offenders(&stripped) {
            all_offenders.push(format!("{}:{line}: {text}", path.display()));
        }
    }
    assert!(
        all_offenders.is_empty(),
        "found production `MaintenanceDialect::DuckDb` hardcode(s) outside a dispatch:\n{}",
        all_offenders.join("\n")
    );
}

#[test]
fn the_scan_flags_a_planted_argument_position_hardcode() {
    let planted = r#"
fn row_fingerprint_expr(cols: &[String], dialect: MaintenanceDialect) -> String {
    let bad = row_fingerprint_expr(cols, MaintenanceDialect::DuckDb);
    bad
}
"#;
    let offenders = find_offenders(planted);
    assert_eq!(
        offenders.len(),
        1,
        "expected exactly one offender, got {offenders:?}"
    );
}

#[test]
fn the_scan_ignores_a_hardcode_inside_a_test_module() {
    let planted = r#"
fn real_emitter(dialect: MaintenanceDialect) -> String {
    match dialect {
        MaintenanceDialect::DuckDb => "ok".to_string(),
        _ => "other".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planted_offender_in_a_test_is_not_a_production_hardcode() {
        let bad = row_fingerprint_expr(&[], MaintenanceDialect::DuckDb);
        assert_eq!(bad, "");
    }
}
"#;
    let stripped = strip_test_modules(planted);
    let offenders = find_offenders(&stripped);
    assert!(
        offenders.is_empty(),
        "expected no offenders after stripping test modules, got {offenders:?}"
    );
}
