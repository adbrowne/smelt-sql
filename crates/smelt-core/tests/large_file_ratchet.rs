use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn script_path() -> PathBuf {
    repo_root().join(".claude/scripts/large-file-check.sh")
}

/// The committed tree must have no tracked file exceeding its baseline and no
/// untracked file exceeding the default cap. One-sided ratchet: a file that
/// shrank below its baseline does NOT need `--update` (see the script's
/// header for why this deliberately differs from hardening-budget.sh).
#[test]
fn gate_passes_on_committed_tree() {
    let root = repo_root();
    let script = script_path();

    assert!(
        script.exists(),
        "large-file-check.sh not found at {script:?}"
    );

    let status = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", &root)
        .current_dir(&root)
        .status()
        .expect("failed to run large-file-check.sh");
    assert!(
        status.success(),
        "large-file-check.sh failed on committed tree.\n\
         • A tracked file grew past its baseline: split it, revert the growth, or\n\
           raise the baseline with a reviewer sign-off note.\n\
         • A new untracked file exceeds the default cap: split it, or register it\n\
           explicitly via `.claude/scripts/large-file-check.sh --update`."
    );
}

/// A tracked file growing past its baseline entry must fail.
#[test]
fn gate_detects_regression_on_tracked_file() {
    let root = repo_root();
    let script = script_path();
    let tempdir = tempfile::tempdir().unwrap();
    let fake_root = tempdir.path();

    let fake_src = fake_root.join("crates/smelt-large-file-probe/src");
    fs::create_dir_all(&fake_src).unwrap();
    // 5 lines now; baseline below claims 3 — a regression.
    fs::write(
        fake_src.join("lib.rs"),
        "pub fn probe() -> i32 {\n    let x = 1;\n    let y = 2;\n    x + y\n}\n",
    )
    .unwrap();

    let fake_claude = fake_root.join(".claude");
    fs::create_dir_all(&fake_claude).unwrap();
    fs::write(
        fake_claude.join("large-file-baseline.txt"),
        "crates/smelt-large-file-probe/src/lib.rs 3\n",
    )
    .unwrap();

    let status = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", fake_root)
        .current_dir(&root)
        .status()
        .expect("failed to run large-file-check.sh on fake tree");
    assert!(
        !status.success(),
        "large-file-check.sh should have detected the tracked-file regression but exited 0"
    );
}

/// A new (unbaselined) file exceeding the default cap must fail.
#[test]
fn gate_detects_new_oversized_file() {
    let root = repo_root();
    let script = script_path();
    let tempdir = tempfile::tempdir().unwrap();
    let fake_root = tempdir.path();

    let fake_src = fake_root.join("crates/smelt-large-file-probe/src");
    fs::create_dir_all(&fake_src).unwrap();
    let mut giant = String::new();
    for i in 0..1600 {
        giant.push_str(&format!("// line {i}\n"));
    }
    fs::write(fake_src.join("lib.rs"), giant).unwrap();

    let fake_claude = fake_root.join(".claude");
    fs::create_dir_all(&fake_claude).unwrap();
    fs::write(fake_claude.join("large-file-baseline.txt"), "#\n").unwrap();

    let status = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", fake_root)
        .current_dir(&root)
        .status()
        .expect("failed to run large-file-check.sh on fake tree");
    assert!(
        !status.success(),
        "large-file-check.sh should have detected the new oversized file but exited 0"
    );
}

/// A tracked file shrinking below its baseline must NOT fail — this is the
/// one-sided behavior that deliberately differs from hardening-budget.sh.
#[test]
fn gate_allows_shrinking_below_baseline() {
    let root = repo_root();
    let script = script_path();
    let tempdir = tempfile::tempdir().unwrap();
    let fake_root = tempdir.path();

    let fake_src = fake_root.join("crates/smelt-large-file-probe/src");
    fs::create_dir_all(&fake_src).unwrap();
    fs::write(fake_src.join("lib.rs"), "pub fn probe() {}\n").unwrap();

    let fake_claude = fake_root.join(".claude");
    fs::create_dir_all(&fake_claude).unwrap();
    // Baseline claims 9000 lines; the file is actually 1 line — a big shrink.
    fs::write(
        fake_claude.join("large-file-baseline.txt"),
        "crates/smelt-large-file-probe/src/lib.rs 9000\n",
    )
    .unwrap();

    let status = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", fake_root)
        .current_dir(&root)
        .status()
        .expect("failed to run large-file-check.sh on fake tree");
    assert!(
        status.success(),
        "large-file-check.sh should allow a tracked file to shrink below baseline without --update"
    );
}

/// A baseline entry whose file no longer exists must fail (unlike an
/// ordinary shrink, a vanished file is a distinct, deliberate event).
#[test]
fn gate_detects_orphaned_baseline_entry() {
    let root = repo_root();
    let script = script_path();
    let tempdir = tempfile::tempdir().unwrap();
    let fake_root = tempdir.path();

    // No crates/ directory at all — the baselined file simply doesn't exist.
    fs::create_dir_all(fake_root.join("crates")).unwrap();
    let fake_claude = fake_root.join(".claude");
    fs::create_dir_all(&fake_claude).unwrap();
    fs::write(
        fake_claude.join("large-file-baseline.txt"),
        "crates/smelt-deleted-crate/src/lib.rs 500\n",
    )
    .unwrap();

    let status = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", fake_root)
        .current_dir(&root)
        .status()
        .expect("failed to run large-file-check.sh on fake tree");
    assert!(
        !status.success(),
        "large-file-check.sh should have detected the orphaned baseline entry but exited 0"
    );
}
