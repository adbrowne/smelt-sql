use std::fs;
use std::path::Path;

use crate::repo_root;

/// Count lines containing the (case-sensitive) needle in a `.rs` file, excluding
/// everything from the first `#[cfg(test)]` line onward.
fn count_needle_in_file(path: &Path, needle: &str) -> usize {
    let content = fs::read_to_string(path).unwrap_or_default();
    let mut count = 0;
    for line in content.lines() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        if line.contains(needle) {
            count += 1;
        }
    }
    count
}

fn count_needle_in_src_dir(dir: &Path, needle: &str) -> usize {
    let mut total = 0;
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total += count_needle_in_src_dir(&path, needle);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            total += count_needle_in_file(&path, needle);
        }
    }
    total
}

/// The retired `refresh: batched` mode's Rust-identifier vocabulary
/// (`BatchedConfig`, `TimeseriesRequiredForBatched`, etc.) must not survive in
/// production `src/` — it was renamed to partition-grain vocabulary
/// (`docs/plans/20260719-prod-w8-composed-axes-followups.md` Phase 4). Lowercase
/// `batched:` (the retired YAML key/mode-value spelling, still named in fix-it
/// text and historical prose) is unaffected — this only guards the capitalised
/// Rust-identifier spelling.
#[test]
fn no_batched_identifier_vocabulary_in_production_src() {
    let root = repo_root();
    let mut failures: Vec<String> = vec![];
    let crates_dir = root.join("crates");
    for entry in fs::read_dir(&crates_dir).expect("crates dir").flatten() {
        let src_dir = entry.path().join("src");
        if !src_dir.exists() {
            continue;
        }
        let count = count_needle_in_src_dir(&src_dir, "Batched");
        if count > 0 {
            failures.push(format!(
                "{}: {count} occurrence(s) of `Batched`",
                entry.file_name().to_string_lossy()
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Production src/ must have zero `Batched` Rust-identifier vocabulary \
         (the retired refresh mode was renamed to partition-grain vocabulary):\n  {}",
        failures.join("\n  ")
    );
}
