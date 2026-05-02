#![cfg(feature = "duckdb")]
//! Regression tests for bug #5 in
//! `docs/plans/20260417-smelt-shop-0.3-followup.md`.
//!
//! `smelt run --dry-run` was silently exiting 0 even when the equivalent live
//! run would fail (parse error, unresolved ref, etc.) and even when the run
//! would have succeeded it printed nothing to stdout. That made dry-run an
//! anti-feature: a green light that means nothing.
//!
//! The two tests below shell out to the compiled `smelt` binary (NOT the
//! library) and assert two complementary properties:
//!
//!   * `dry_run_surfaces_unresolved_ref_errors` — a workspace with one
//!     unresolved `smelt.ref()` exits non-zero with a useful stderr message.
//!     Captures the "silent success on broken workspace" half of the bug.
//!
//!   * `dry_run_prints_planned_sql_per_model` — a valid workspace prints a
//!     "Would run: <model>" header and the compiled SQL to stdout for every
//!     model in execution order. Captures the "silent success on healthy
//!     workspace" half — even when nothing is wrong, dry-run must show what
//!     it *would* do.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Locate the compiled `smelt` binary. Cargo sets `CARGO_BIN_EXE_<name>` for
/// integration tests in the package that defines the binary.
fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

/// Write a minimal valid `smelt.yml` for a DuckDB target into `dir`.
fn write_smelt_yml(dir: &Path, name: &str) {
    let yml = format!(
        "name: {name}\n\
         version: 1\n\
         model_paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n"
    );
    std::fs::write(dir.join("smelt.yml"), yml).unwrap();
}

/// Stage a workspace tree and return its root path. The tempdir is owned by
/// the caller to keep it alive for the duration of the test.
fn stage_workspace(tmp: &TempDir, name: &str, model_files: &[(&str, &str)]) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join("models")).unwrap();
    write_smelt_yml(&root, name);
    for (file, content) in model_files {
        std::fs::write(root.join("models").join(file), content).unwrap();
    }
    root
}

/// Run `smelt run --dry-run [--verbose]` against `project_dir`. Returns the
/// raw `Output` so each test can assert on exit status / stdout / stderr.
fn run_dry_run(project_dir: &Path, verbose: bool) -> std::process::Output {
    let mut cmd = Command::new(smelt_bin());
    cmd.args([
        "run",
        "--dry-run",
        "--project-dir",
        project_dir.to_str().unwrap(),
    ]);
    if verbose {
        cmd.arg("--verbose");
    }
    // Exercise without any RUST_LOG forcing — the bug was that the user had
    // no log filter set and saw nothing. Stdout must carry the contract,
    // not the tracing layer.
    cmd.env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run --dry-run`: {e}"))
}

#[test]
fn dry_run_surfaces_unresolved_ref_errors() {
    // A model that references an undefined upstream using path form.
    // The live run would fail at dependency validation; dry-run must mirror that.
    let tmp = TempDir::new().unwrap();
    let workspace = stage_workspace(
        &tmp,
        "broken_dry_run",
        // Phase 4: use path form smelt.models.does_not_exist instead of
        // smelt.models.does_not_exist — the legacy form now produces a parse error.
        &[("bad.sql", "SELECT * FROM smelt.models.does_not_exist\n")],
    );

    let output = run_dry_run(&workspace, true);

    assert!(
        !output.status.success(),
        "dry-run with unresolved ref must exit non-zero (got {:?}); \
         stderr:\n{}\nstdout:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    // The error message must mention the offending model.
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(
        combined.contains("bad"),
        "stderr must mention the broken model 'bad'; got:\n{combined}"
    );
}

#[test]
fn dry_run_prints_planned_sql_per_model() {
    // Two-model workspace: a leaf (`base`) and a downstream (`derived`) that
    // refs the leaf. Dry-run should print a header for each in execution
    // order and exit 0.
    let tmp = TempDir::new().unwrap();
    let workspace = stage_workspace(
        &tmp,
        "valid_dry_run",
        &[
            ("base.sql", "SELECT 1 AS x\n"),
            // Phase 4: use path form instead of smelt.models.base.
            ("derived.sql", "SELECT x + 1 AS y FROM smelt.models.base\n"),
        ],
    );

    let output = run_dry_run(&workspace, true);

    assert!(
        output.status.success(),
        "dry-run on valid workspace must exit 0 (got {:?}); \
         stderr:\n{}\nstdout:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // A header line per model proves dry-run actually walked the plan and
    // emitted the work it would have done — not just exited silently.
    assert!(
        stdout.contains("Would run: base"),
        "dry-run stdout must include a 'Would run: base' header; got:\n{stdout}"
    );
    assert!(
        stdout.contains("Would run: derived"),
        "dry-run stdout must include a 'Would run: derived' header; got:\n{stdout}"
    );
    // The compiled SQL for `derived` must include the resolved ref (qualified
    // table name in the configured schema), proving compilation actually ran.
    assert!(
        stdout.contains("main.base"),
        "dry-run stdout must include the resolved table name `main.base` in \
         the compiled SQL for `derived`; got:\n{stdout}"
    );
}
