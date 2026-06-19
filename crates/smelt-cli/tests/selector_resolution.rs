//! Integration tests for selector `+` graph operators (D-38): strip before
//! entity resolution, re-attach to the resolved full path.
//!
//! Spec: `docs/specs/cli.md` §"Argument resolution algorithm" (graph operators)
//! and `docs/specs/model_selection.md` §"Selection methods".

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_smelt_yml(dir: &Path, name: &str) {
    let yml = format!(
        "name: {name}\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n"
    );
    fs::write(dir.join("smelt.yml"), yml).unwrap();
}

/// Stage a two-model workspace: `base` (leaf) and `derived` (depends on base).
/// Returns the project root path.
fn stage_base_derived(tmp: &TempDir, name: &str) -> PathBuf {
    let root = tmp.path().join(name);
    fs::create_dir_all(root.join("models")).unwrap();
    write_smelt_yml(&root, name);
    fs::write(root.join("models").join("base.sql"), "SELECT 1 AS x\n").unwrap();
    fs::write(
        root.join("models").join("derived.sql"),
        "SELECT x + 1 AS y FROM smelt.base\n",
    )
    .unwrap();
    root
}

fn run_dry(project_dir: &Path, select: &str) -> std::process::Output {
    Command::new(smelt_bin())
        .args(["run", "--project-dir", project_dir.to_str().unwrap()])
        .args(["--select", select, "--dry-run"])
        .env_remove("RUST_LOG")
        .output()
        .expect("smelt binary should be runnable")
}

// ── D-38: `+` graph operators stripped before resolution, re-attached after ──

/// `+derived` strips the `+` before entity resolution → resolves `derived` →
/// re-attaches `+` → upstream operator preserved → `base` (the upstream) is
/// included in the dry-run set (D-38).
#[test]
fn plus_prefix_resolves_then_reattaches() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "plus_prefix");

    let output = run_dry(&root, "+derived");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "--select +derived should succeed (+ is stripped before resolution)\nstderr: {stderr}\nstdout: {stdout}"
    );
    // Upstream `base` is included because the `+` upstream flag is preserved.
    assert!(
        stdout.contains("Would run: base"),
        "--select +derived should include upstream 'base' (operator preserved)\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("Would run: derived"),
        "--select +derived should include 'derived' itself\nstdout: {stdout}"
    );
}

/// `base+` preserves the trailing `+` downstream operator through entity
/// resolution → downstream `derived` is included (D-38).
#[test]
fn plus_suffix_resolves_then_reattaches() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "plus_suffix");

    let output = run_dry(&root, "base+");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "--select base+ should succeed\nstderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: base"),
        "--select base+ should include 'base'\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: derived"),
        "--select base+ (downstream) should include downstream 'derived'\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// `+base+` — both upstream and downstream operators are preserved through
/// resolution: the whole graph is selected (D-38).
#[test]
fn plus_both_resolves_and_traverses_all() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "plus_both");

    let output = run_dry(&root, "+base+");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "--select +base+ should succeed\nstderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: base"),
        "--select +base+ should include 'base'\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: derived"),
        "--select +base+ should include downstream 'derived'\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// `path:` is NOT a recognised selection method — a `path:models/silver`
/// selector is treated as a model-name reference that fails to resolve,
/// confirming no `path:` method was added (D-38).
#[test]
fn no_path_method_path_colon_errors_on_resolution() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "no_path_method");

    let output = run_dry(&root, "path:models/silver");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "path:models/silver should fail — not a recognised method (treated as model name which does not exist)\nstderr: {stderr}\nstdout: {stdout}"
    );
    // The error should be a resolution failure, not a silent "0 models matched".
    assert!(
        stderr.contains("path:models/silver") || stderr.contains("not found"),
        "stderr should indicate the unresolvable selector; got:\n{stderr}"
    );
}
