#![cfg(feature = "duckdb")]
//! Phase 1 of `docs/plans/20260503-loop-medium-followups.md`:
//! `smelt build` must reject `smelt.<path>(...)` call forms where the
//! path includes the declaring file's stem (e.g.
//! `smelt.functions.status.is_shipped(...)` where `status` is the stem of
//! `functions/status.sql`).
//!
//! Spec rule (`docs/specs/functions.md` §"Function call syntax"):
//!   The filename stem is **not** a path component.
//!   `functions/status.sql` declaring `is_shipped` → called as
//!   `smelt.functions.is_shipped(...)`, NOT
//!   `smelt.functions.status.is_shipped(...)`.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

const STATUS_FN_SRC: &str =
    "smelt.define is_shipped(status: Expr<Text>) -> Expr<Boolean> AS (status = 'shipped')\n";

/// Stage a minimal smelt workspace under `tmp/<name>/`.
///
/// Creates:
///   - `smelt.yml` (duckdb target, view materialization)
///   - `models/` directory (populated from `model_files`)
///   - `functions/` directory (populated from `fn_files`)
///   - `seeds/` and `target/` directories
fn stage_workspace(
    tmp: &TempDir,
    name: &str,
    model_files: &[(&str, &str)],
    fn_files: &[(&str, &str)],
) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("seeds")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();

    let yml = format!(
        "name: {name}\n\
         version: 1\n\
         model_paths:\n  - models\n\
         seed_paths:\n  - seeds\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n"
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();

    for (file, content) in model_files {
        std::fs::write(root.join("models").join(file), content).unwrap();
    }

    for (rel_path, content) in fn_files {
        let dest = root.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&dest, content).unwrap();
    }

    root
}

fn run_build(project_dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"))
}

fn run_show_plan(project_dir: &Path, model_rel: &str) -> std::process::Output {
    let model_abs = project_dir.join(model_rel);
    Command::new(smelt_bin())
        .args([
            "build",
            model_abs.to_str().unwrap(),
            "--show-plan",
            "--project-dir",
            project_dir.to_str().unwrap(),
        ])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build --show-plan`: {e}"))
}

/// Test 1 — `smelt build` must exit non-zero when a model uses a stem-included
/// call path (`smelt.functions.status.is_shipped(...)`).
/// The declaring file is `functions/status.sql`; the stem `status` is NOT a
/// path component per the spec.
#[test]
fn test_build_rejects_stem_in_call_path() {
    let tmp = TempDir::new().unwrap();
    // Model calls `smelt.functions.status.is_shipped(s)` — stem `status` included.
    let workspace = stage_workspace(
        &tmp,
        "build_rejects_stem",
        &[(
            "stg.sql",
            "SELECT smelt.functions.status.is_shipped('shipped') AS r\n",
        )],
        &[("functions/status.sql", STATUS_FN_SRC)],
    );

    let output = run_build(&workspace);

    assert!(
        !output.status.success(),
        "smelt build must exit non-zero when a call includes the file stem in the path; \
         got exit 0.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // The error message should name the wrong call path.
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("is_shipped") || combined.contains("UnknownSmeltFn"),
        "error output should reference the offending function call; got:\n{combined}"
    );
}

/// Test 2 — `smelt build` must exit 0 when a model uses the canonical no-stem
/// call path (`smelt.functions.is_shipped(...)`).
#[test]
fn test_build_accepts_canonical_no_stem_call_path() {
    let tmp = TempDir::new().unwrap();
    // Model calls `smelt.functions.is_shipped(s)` — no stem, canonical form.
    let workspace = stage_workspace(
        &tmp,
        "build_accepts_canonical",
        &[(
            "stg.sql",
            "SELECT smelt.functions.is_shipped('shipped') AS r\n",
        )],
        &[("functions/status.sql", STATUS_FN_SRC)],
    );

    let output = run_build(&workspace);

    assert!(
        output.status.success(),
        "smelt build must exit 0 for canonical no-stem call path; \
         got {:?}.\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Test 3 — `smelt build --show-plan` must exit non-zero when the model uses
/// a stem-included call path.
#[test]
fn test_show_plan_rejects_stem_in_call_path() {
    let tmp = TempDir::new().unwrap();
    let workspace = stage_workspace(
        &tmp,
        "show_plan_rejects_stem",
        &[(
            "stg.sql",
            "SELECT smelt.functions.status.is_shipped('shipped') AS r\n",
        )],
        &[("functions/status.sql", STATUS_FN_SRC)],
    );

    let output = run_show_plan(&workspace, "models/stg.sql");

    assert!(
        !output.status.success(),
        "smelt build --show-plan must exit non-zero when a call includes the file stem; \
         got exit 0.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
