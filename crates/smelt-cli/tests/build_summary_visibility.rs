#![cfg(feature = "duckdb")]
//! Regression tests for issue #2 in iter-3/iter-4 of the smelt-shop 0.3
//! follow-up: a successful `smelt build` produced zero stdout/stderr output,
//! making it indistinguishable from "did nothing", "hung", or "succeeded"
//! when the user had no `RUST_LOG` filter set. Mirroring a one-line summary
//! to stderr unconditionally restores user-visible feedback on success.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_smelt_yml(dir: &Path, name: &str) {
    let yml = format!(
        "name: {name}\n\
         version: 1\n\
         model_paths:\n  - models\n\
         seed_paths:\n  - seeds\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n"
    );
    std::fs::write(dir.join("smelt.yml"), yml).unwrap();
}

fn stage_workspace(tmp: &TempDir, name: &str, model_files: &[(&str, &str)]) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("seeds")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    write_smelt_yml(&root, name);
    for (file, content) in model_files {
        std::fs::write(root.join("models").join(file), content).unwrap();
    }
    root
}

fn run_build(project_dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .args(["build", "--project-dir", project_dir.to_str().unwrap()])
        // No RUST_LOG: this is the contract being pinned. Users without a
        // log filter must still see one line confirming the run happened.
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"))
}

#[test]
fn build_emits_run_summary_to_stderr_on_success() {
    let tmp = TempDir::new().unwrap();
    let workspace = stage_workspace(
        &tmp,
        "build_summary_ws",
        &[
            ("base.sql", "SELECT 1 AS x\n"),
            ("derived.sql", "SELECT x + 1 AS y FROM smelt.ref('base')\n"),
        ],
    );

    let output = run_build(&workspace);

    assert!(
        output.status.success(),
        "smelt build on valid workspace must exit 0 (got {:?}); \
         stderr:\n{}\nstdout:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    // The one-line summary must include a count of models built. Pinning the
    // exact prefix `smelt: built` keeps the contract greppable for users and
    // CI scripts; the count proves the run actually walked the graph and
    // wasn't a silent no-op.
    assert!(
        stderr.contains("smelt: built 2 model(s)"),
        "stderr must include 'smelt: built 2 model(s)' summary on success; \
         got stderr:\n{stderr}"
    );
}
