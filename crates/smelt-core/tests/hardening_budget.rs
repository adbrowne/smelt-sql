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

/// A crate used only as a `[dev-dependencies]` entry is test-support, not
/// production, and its `unwrap`/`expect` debt must not enter the baseline.
///
/// The rule is *derived*, not declared: a crate counts as test-support only
/// when some crate names it under `[dev-dependencies]` AND no crate names it
/// under `[dependencies]`. That shape matters — "nothing depends on it" alone
/// would wrongly exclude every top-level binary crate (`smelt-cli`,
/// `smelt-ui`, …), which nothing depends on either. Deriving it means that
/// promoting a testkit to a real dependency silently re-enters its debt into
/// the gate, where a hand-maintained exclusion list would have gone stale.
#[test]
fn dev_only_crates_are_excluded_from_the_budget() {
    let tempdir = tempfile::tempdir().unwrap();
    let fake_root = tempdir.path();

    let write = |rel: &str, body: &str| {
        let path = fake_root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    };

    // `app` — a top-level crate nothing depends on. Still production.
    write(
        "crates/app/Cargo.toml",
        "[package]\nname = \"app\"\n\n[dependencies]\nprod-lib = { path = \"../prod-lib\" }\n\n\
         [dev-dependencies]\ntest-kit = { path = \"../test-kit\" }\n",
    );
    write("crates/app/src/lib.rs", "pub fn ok() {}\n");

    // `prod-lib` — a real dependency of `app`. Its one unwrap is production debt.
    write(
        "crates/prod-lib/Cargo.toml",
        "[package]\nname = \"prod-lib\"\n",
    );
    write(
        "crates/prod-lib/src/lib.rs",
        "pub fn probe() -> i32 { let x: Option<i32> = None; x.unwrap() }\n",
    );

    // `tool` — a shipped binary that some crate dev-depends on via a test-only
    // back edge (`smelt-runtime` does exactly this to `smelt-cli`). Nothing
    // depends on it normally, so the dev-dep half of the rule alone would
    // wrongly excuse it. A crate with a binary target ships to users and stays
    // production; its debt must still be counted.
    write(
        "crates/tool/Cargo.toml",
        "[package]\nname = \"tool\"\n\n[dev-dependencies]\ntest-kit = { path = \"../test-kit\" }\n",
    );
    write(
        "crates/tool/src/main.rs",
        "fn main() { println!(\"hi\"); }\n",
    );
    write(
        "crates/tool/src/lib.rs",
        "pub fn probe() -> i32 { let x: Option<i32> = None; x.unwrap() }\n",
    );
    write(
        "crates/app-that-dev-deps-tool/Cargo.toml",
        "[package]\nname = \"app-that-dev-deps-tool\"\n\n\
         [dev-dependencies]\ntool = { path = \"../tool\" }\n",
    );
    write(
        "crates/app-that-dev-deps-tool/src/lib.rs",
        "pub fn ok() {}\n",
    );

    // `test-kit` — dev-dependency of `app`, regular dependency of nobody,
    // and no binary target. Its debt must be invisible to the gate.
    write(
        "crates/test-kit/Cargo.toml",
        "[package]\nname = \"test-kit\"\n",
    );
    write(
        "crates/test-kit/src/lib.rs",
        "pub fn a() -> i32 { let x: Option<i32> = None; x.unwrap() }\n\
         pub fn b() -> i32 { let x: Option<i32> = None; x.unwrap() }\n\
         pub fn c() -> i32 { let x: Option<i32> = None; x.expect(\"c\") }\n",
    );

    // Baseline registers every production crate — `tool` included, with the
    // debt its binary target keeps in scope — but not `test-kit`. The gate is
    // a two-sided ratchet, so an over-broad exclusion shows up here as a
    // "unregistered crate" error and an under-broad one as a stale-baseline
    // error; either way this assertion fails.
    write(
        ".claude/hardening-baseline.txt",
        "app unwrap 0\napp expect 0\napp println 0\n\
         app-that-dev-deps-tool unwrap 0\napp-that-dev-deps-tool expect 0\n\
         app-that-dev-deps-tool println 0\n\
         prod-lib unwrap 1\nprod-lib expect 0\nprod-lib println 0\n\
         tool unwrap 1\ntool expect 0\ntool println 1\n",
    );

    let output = Command::new("bash")
        .arg(script_path())
        .env("REPO_ROOT", fake_root)
        .current_dir(repo_root())
        .output()
        .expect("failed to run hardening-budget.sh on fake tree");

    assert!(
        output.status.success(),
        "gate should ignore the dev-only `test-kit` crate, but failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
