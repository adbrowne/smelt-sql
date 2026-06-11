use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

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

fn script_path() -> PathBuf {
    repo_root().join(".claude/scripts/hardening-budget.sh")
}

/// The committed tree must exactly match the baseline — two-sided ratchet.
/// Also verifies that a regression (extra .unwrap() injected) is detected.
#[test]
fn gate_detects_regression() {
    let root = repo_root();
    let script = script_path();

    assert!(
        script.exists(),
        "hardening-budget.sh not found at {script:?} — Phase 1 not yet implemented"
    );

    // ── Test A: committed tree exits 0 ────────────────────────────────────────
    // If this fails, either the tree regressed (count > baseline) or the
    // baseline is stale because debt was paid (count < baseline).
    let status = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", &root)
        .current_dir(&root)
        .status()
        .expect("failed to run hardening-budget.sh");
    assert!(
        status.success(),
        "hardening-budget.sh failed on committed tree.\n\
         • If production debt grew: revert or fix the regression.\n\
         • If debt shrank (good!): run `.claude/scripts/hardening-budget.sh --update` \
           to tighten the baseline."
    );

    // ── Test B: injected .unwrap() is detected ────────────────────────────────
    let tempdir = tempfile::tempdir().unwrap();
    let fake_root = tempdir.path();

    // Minimal fake crate with one .unwrap()
    let fake_src = fake_root.join("crates/smelt-hardening-probe/src");
    fs::create_dir_all(&fake_src).unwrap();
    fs::write(
        fake_src.join("lib.rs"),
        "pub fn probe() -> i32 { let x: Option<i32> = None; x.unwrap() }\n",
    )
    .unwrap();

    // Baseline claims 0 unwrap → the injected one is a regression
    let fake_claude = fake_root.join(".claude");
    fs::create_dir_all(&fake_claude).unwrap();
    fs::write(
        fake_claude.join("hardening-baseline.txt"),
        "smelt-hardening-probe unwrap 0\n\
         smelt-hardening-probe expect 0\n\
         smelt-hardening-probe println 0\n",
    )
    .unwrap();

    let status = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", fake_root)
        .current_dir(&root)
        .status()
        .expect("failed to run hardening-budget.sh on fake tree");
    assert!(
        !status.success(),
        "hardening-budget.sh should have detected the injected regression but exited 0"
    );
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
