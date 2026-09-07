//! Source-text gate: every `println!`/`eprintln!` site added to `smelt-cli` since the
//! 2026-08/09 hardening burst carries an in-source `// stdout: <reason>` marker, and the
//! one added `.expect(` carries an `// invariant: <reason>` marker. See
//! `docs/outcomes/20260904-ratchet-paydown/phases/03-plan.md`.

use std::fs;
use std::path::Path;

fn lines(path: &str) -> Vec<String> {
    let full = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    fs::read_to_string(&full)
        .unwrap_or_else(|e| panic!("failed to read {full:?}: {e}"))
        .lines()
        .map(|s| s.to_string())
        .collect()
}

/// Walks upward from `idx` (exclusive), collecting a contiguous run of comment lines
/// (skipping blank lines), and reports whether any collected line starts with `marker`.
fn marker_precedes(lines: &[String], idx: usize, marker: &str) -> bool {
    let mut i = idx;
    let mut found = false;
    while i > 0 {
        i -= 1;
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("//") {
            if trimmed.starts_with(marker) {
                found = true;
            }
            continue;
        }
        break;
    }
    found
}

/// Finds the line index of the `println!(`/`eprintln!(` call that (eventually) contains
/// the statement starting at or after `from_idx`, searching upward.
fn enclosing_call_start(lines: &[String], from_idx: usize) -> usize {
    let mut i = from_idx;
    loop {
        let trimmed = lines[i].trim_start();
        if trimmed.starts_with("println!") || trimmed.starts_with("eprintln!") {
            return i;
        }
        if i == 0 {
            panic!(
                "no enclosing println!/eprintln! found above line {}",
                from_idx + 1
            );
        }
        i -= 1;
    }
}

#[test]
fn migrate_command_stdout_sites_are_marked() {
    let path = "src/commands/migrate.rs";
    let src = lines(path);
    let mut unmarked = Vec::new();
    for (idx, line) in src.iter().enumerate() {
        if line.trim_start().starts_with("println!") && !marker_precedes(&src, idx, "// stdout:") {
            unmarked.push(idx + 1);
        }
    }
    assert!(
        unmarked.is_empty(),
        "{path}: println! sites missing a `// stdout:` marker on line(s): {unmarked:?}"
    );
}

#[test]
fn state_mode_and_selector_stdout_sites_are_marked() {
    let cases: &[(&str, &str)] = &[
        ("src/commands/history.rs", "state.mode: stateless"),
        ("src/commands/status.rs", "state.mode: stateless"),
    ];
    for (path, substring) in cases {
        let src = lines(path);
        let idx = src
            .iter()
            .position(|l| l.contains(substring))
            .unwrap_or_else(|| panic!("{path}: no line containing {substring:?}"));
        let call_idx = enclosing_call_start(&src, idx);
        assert!(
            marker_precedes(&src, call_idx, "// stdout:"),
            "{path}: println! site containing {substring:?} (line {}) missing a `// stdout:` marker",
            call_idx + 1
        );
    }

    // run.rs has two occurrences of the same message; only the second (the
    // --since-upstream intersection path) is required to carry a marker.
    let path = "src/commands/run.rs";
    let src = lines(path);
    let occurrences: Vec<usize> = src
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("smelt: no models matched the selector(s)"))
        .map(|(i, _)| i)
        .collect();
    assert!(
        occurrences.len() >= 2,
        "{path}: expected at least 2 occurrences of the 'no models matched' message, found {}",
        occurrences.len()
    );
    let second = enclosing_call_start(&src, occurrences[1]);
    assert!(
        marker_precedes(&src, second, "// stdout:"),
        "{path}: second 'no models matched' site (line {}) missing a `// stdout:` marker",
        second + 1
    );
}

#[test]
fn migrate_json_expect_is_justified() {
    let path = "src/commands/migrate.rs";
    let src = lines(path);
    let idx = src
        .iter()
        .position(|l| l.contains(".expect(\"JSON serialization should not fail\")"))
        .unwrap_or_else(|| panic!("{path}: expected JSON serialization expect() site not found"));
    let call_idx = enclosing_call_start(&src, idx);
    assert!(
        marker_precedes(&src, call_idx, "// invariant:"),
        "{path}: JSON serialization expect() (enclosing call at line {}) missing an `// invariant:` marker",
        call_idx + 1
    );
}
