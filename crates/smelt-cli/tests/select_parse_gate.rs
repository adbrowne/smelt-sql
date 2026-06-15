//! Tests for parse-error gating scoped to the `--select` subgraph.
//!
//! A broken unrelated model must not abort `smelt run --select <good_model>
//! --dry-run`. Errors inside the selected subgraph (including transitive deps)
//! must still block. The whole-workspace gate must be preserved when no
//! `--select` is given.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_smelt_yml(dir: &Path) {
    let yml = "name: parse_gate_test\n\
               version: 1\n\
               paths:\n  - models\n\
               targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
               default_materialization: view\n";
    std::fs::write(dir.join("smelt.yml"), yml).unwrap();
}

fn stage_workspace(tmp: &TempDir, model_files: &[(&str, &str)]) -> PathBuf {
    let root = tmp.path().to_path_buf();
    std::fs::create_dir_all(root.join("models")).unwrap();
    write_smelt_yml(&root);
    for (file, content) in model_files {
        std::fs::write(root.join("models").join(file), content).unwrap();
    }
    root
}

fn run_dry(project_dir: &Path, select: Option<&str>) -> (bool, String) {
    let mut cmd = Command::new(smelt_bin());
    cmd.arg("run")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .arg("--dry-run");
    if let Some(sel) = select {
        cmd.args(["--select", sel]);
    }
    cmd.env_remove("RUST_LOG");
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt: {e}"));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

/// `broken.sql` has a genuine parse error: missing select-list items force the
/// parser to emit an error node.
const BROKEN_SQL: &str = "SELECT FROM events\n";

/// `good.sql` is a clean, self-contained model.
const GOOD_SQL: &str = "SELECT 1 AS x\n";

/// `downstream.sql` depends on `broken` via a smelt ref.
const DOWNSTREAM_SQL: &str = "SELECT * FROM smelt.broken\n";

#[test]
fn select_skips_unrelated_broken_model() {
    // Workspace: broken.sql (parse error) + good.sql (clean, no deps).
    // `smelt run --select good --dry-run` must succeed because the broken
    // model is outside the selected subgraph.
    let tmp = TempDir::new().unwrap();
    let root = stage_workspace(&tmp, &[("broken.sql", BROKEN_SQL), ("good.sql", GOOD_SQL)]);

    let (ok, output) = run_dry(&root, Some("good"));
    assert!(
        ok,
        "--select good should succeed when the only broken model is unrelated;\
         \ncombined output:\n{output}"
    );
}

#[test]
fn select_blocks_when_dep_has_parse_error() {
    // Workspace: broken.sql (parse error) + downstream.sql (refs broken).
    // `smelt run --select downstream --dry-run` must fail because the
    // broken model is a transitive dep of the selected model.
    let tmp = TempDir::new().unwrap();
    let root = stage_workspace(
        &tmp,
        &[
            ("broken.sql", BROKEN_SQL),
            ("downstream.sql", DOWNSTREAM_SQL),
        ],
    );

    let (ok, output) = run_dry(&root, Some("downstream"));
    assert!(
        !ok,
        "--select downstream should fail when its dep `broken` has parse errors;\
         \ncombined output:\n{output}"
    );
}

#[test]
fn no_select_blocks_on_any_broken_model() {
    // Workspace: broken.sql (parse error) + good.sql (clean).
    // `smelt run --dry-run` (no --select) must fail because a broken model
    // is present in the full workspace.
    let tmp = TempDir::new().unwrap();
    let root = stage_workspace(&tmp, &[("broken.sql", BROKEN_SQL), ("good.sql", GOOD_SQL)]);

    let (ok, output) = run_dry(&root, None);
    assert!(
        !ok,
        "smelt run without --select should fail when any model has parse errors;\
         \ncombined output:\n{output}"
    );
}
