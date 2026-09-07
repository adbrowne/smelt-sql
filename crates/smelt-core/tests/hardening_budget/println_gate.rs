use std::fs;
use std::path::Path;

use crate::repo_root;

/// Count lines containing "println!" in the production (pre-cfg-test) portion of a file.
/// Excludes main.rs (binary entry points) and tests.rs files; the caller excludes tests/ dirs.
/// Uses substring matching: "println!" matches both `println!` and `eprintln!`.
fn count_println_in_file(path: &Path) -> usize {
    let fname = path.file_name().unwrap_or_default();
    if fname == "tests.rs" || fname == "main.rs" {
        return 0;
    }
    let content = fs::read_to_string(path).unwrap_or_default();
    let mut count = 0;
    for line in content.lines() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        if line.contains("println!") {
            count += 1;
        }
    }
    count
}

fn count_println_in_src_dir(dir: &Path) -> usize {
    let mut total = 0;
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().map(|n| n == "tests").unwrap_or(false) {
                continue; // skip tests/ subdirectory
            }
            total += count_println_in_src_dir(&path);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            total += count_println_in_file(&path);
        }
    }
    total
}

/// Library crates must have zero production println!/eprintln!.
/// main.rs (binary entry points) and test code are excluded.
/// Legitimate CLI output (smelt-cli, smelt-ui, smelt-bench, smelt-datagen main.rs) stays.
#[test]
fn no_println_in_libraries() {
    let root = repo_root();
    // Crates whose entire library surface must be println!-free.
    // smelt-datagen is included: its main.rs (binary output) is excluded automatically.
    let library_crates = &[
        "smelt-db",
        "smelt-types",
        "smelt-parser",
        "smelt-planner",
        "smelt-logical",
        "smelt-runtime",
        "smelt-dialect",
        "smelt-state",
        "smelt-datagen",
        "smelt-core",
        "smelt-backend-duckdb",
        "smelt-backend-spark",
        "smelt-backend",
        "smelt-parser-compat",
    ];

    let mut failures: Vec<String> = vec![];
    for &crate_name in library_crates {
        let src_dir = root.join("crates").join(crate_name).join("src");
        if !src_dir.exists() {
            continue;
        }
        let count = count_println_in_src_dir(&src_dir);
        if count > 0 {
            failures.push(format!(
                "{crate_name}: {count} println!/eprintln! in library code (migrate to tracing::warn!/debug!)"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Library crates must have zero production println!/eprintln!:\n  {}\n\
         Migrate these to tracing::warn! / tracing::debug! and lower the println baseline.",
        failures.join("\n  ")
    );
}
