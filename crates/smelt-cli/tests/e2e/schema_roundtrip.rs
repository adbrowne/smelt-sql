#![cfg(feature = "duckdb")]
//! Phase 1 & Phase 2 regression tests from docs/plans/20260505-smelt-state-cli-bugfixes.md.
//!
//! Phase 1: build → diff must show no phantom ChangeNullability on an unchanged model.
//! Phase 2: build → delete model file → rebuild → stale schema entry is removed.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn setup_workspace(dir: &Path) {
    std::fs::create_dir_all(dir.join("models")).unwrap();
    std::fs::create_dir_all(dir.join("seeds")).unwrap();

    std::fs::write(
        dir.join("smelt.yml"),
        r#"
name: roundtrip-test
version: 1
paths:
  - models
  - seeds
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
state:
  mode: intervals
"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("seeds/raw_orders.csv"),
        "order_id,customer_id,amount\n1,100,29.99\n2,101,49.99\n3,100,19.99\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("models/stg_orders.sql"),
        "---\nname: stg_orders\nmaterialization: table\n---\n\
         SELECT order_id, customer_id, amount FROM smelt.raw_orders\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("models/mart_summary.sql"),
        "---\nname: mart_summary\nmaterialization: table\n---\n\
         SELECT customer_id, COUNT(*) AS order_count, SUM(amount) AS total_amount \
         FROM smelt.stg_orders GROUP BY 1\n",
    )
    .unwrap();
}

fn run_smelt(args: &[&str], dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .args(args)
        .arg("--project-dir")
        .arg(dir.to_str().unwrap())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt: {e}"))
}

/// Phase 1: `smelt build` followed by `smelt diff` must show no schema changes.
///
/// Regression for the phantom ChangeNullability bug: if type inference is not
/// deterministic between the save path (run.rs) and the diff path (diff.rs),
/// smelt diff would report spurious nullability changes.
#[test]
fn no_phantom_nullability_after_clean_build() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_workspace(dir);

    let build = run_smelt(&["build"], dir);
    assert!(
        build.status.success(),
        "smelt build failed:\nstderr: {}\nstdout: {}",
        String::from_utf8_lossy(&build.stderr),
        String::from_utf8_lossy(&build.stdout),
    );

    // Schema files must exist
    assert!(dir
        .join(".smelt/targets/dev/schemas/stg_orders.json")
        .exists());
    assert!(dir
        .join(".smelt/targets/dev/schemas/mart_summary.json")
        .exists());

    // smelt diff must exit 0 (no changes detected)
    let diff = run_smelt(&["diff"], dir);
    let stderr = String::from_utf8_lossy(&diff.stderr);
    let stdout = String::from_utf8_lossy(&diff.stdout);
    assert!(
        diff.status.success(),
        "smelt diff reported changes after a clean build (phantom nullability?):\nstdout: {stdout}\nstderr: {stderr}",
    );
    assert!(
        stdout.contains("No schema changes detected"),
        "expected 'No schema changes detected', got:\n{stdout}",
    );
}

/// Phase 2: after deleting a model file and rebuilding, the stale schema entry
/// must be removed from `.smelt/targets/dev/schemas/`.
#[test]
fn stale_schema_cleaned_after_model_deleted() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_workspace(dir);

    // Build once — both schema files appear
    let build1 = run_smelt(&["build"], dir);
    assert!(
        build1.status.success(),
        "first smelt build failed:\nstderr: {}",
        String::from_utf8_lossy(&build1.stderr),
    );
    assert!(dir
        .join(".smelt/targets/dev/schemas/mart_summary.json")
        .exists());

    // Delete mart_summary.sql
    std::fs::remove_file(dir.join("models/mart_summary.sql")).unwrap();

    // Rebuild — mart_summary is no longer in the project
    let build2 = run_smelt(&["build"], dir);
    assert!(
        build2.status.success(),
        "second smelt build failed:\nstderr: {}",
        String::from_utf8_lossy(&build2.stderr),
    );

    // Stale schema file must be removed
    assert!(
        !dir.join(".smelt/targets/dev/schemas/mart_summary.json")
            .exists(),
        "stale schema file was not cleaned up after model deletion"
    );

    // smelt diff must exit 0 — no phantom REMOVED entry
    let diff = run_smelt(&["diff"], dir);
    let stdout = String::from_utf8_lossy(&diff.stdout);
    assert!(
        diff.status.success(),
        "smelt diff reports REMOVED after stale schema cleanup:\nstdout: {stdout}",
    );
}

/// Sub-directory models must participate in schema evolution exactly like flat
/// models. Schemas are keyed by the db-name (`address_segments.join("_")`, e.g.
/// `staging_stg_orders`) in the save + migration paths; `smelt diff` and the
/// stale-schema cleanup must agree on that key.
///
/// Regression for the canonical-vs-db_name asymmetry (BUG-034/035):
///   1. The just-saved `staging_stg_orders.json` was deleted by the stale-schema
///      cleanup on the same run (cleanup compared db-name file stems against the
///      canonical `all_model_names()` set), so a sub-dir model's schema never
///      persisted and schema evolution silently never triggered.
///   2. `smelt diff` skipped sub-dir models entirely (model_lookup keyed by the
///      leaf `m.name`, but the iteration used canonical paths → lookup miss →
///      `continue`), so a sub-dir model was never reported (`0 models checked`).
#[test]
fn subdir_model_schema_persists_and_diffs() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("models/staging")).unwrap();
    std::fs::create_dir_all(dir.join("seeds")).unwrap();

    std::fs::write(
        dir.join("smelt.yml"),
        r#"
name: subdir-test
version: 1
paths:
  - models
  - seeds
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
state:
  mode: intervals
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("seeds/raw_orders.csv"),
        "order_id,customer_id,amount\n1,100,29.99\n2,101,49.99\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("models/staging/stg_orders.sql"),
        "---\nmaterialization: table\n---\n\
         SELECT order_id, customer_id, amount FROM smelt.raw_orders\n",
    )
    .unwrap();

    let build = run_smelt(&["build"], dir);
    assert!(
        build.status.success(),
        "smelt build failed:\nstderr: {}",
        String::from_utf8_lossy(&build.stderr),
    );

    // The schema for the sub-dir model must survive the stale-schema cleanup.
    assert!(
        dir.join(".smelt/targets/dev/schemas/staging_stg_orders.json")
            .exists(),
        "sub-dir model schema was deleted by stale-schema cleanup; \
         .smelt/targets/dev/schemas/ = {:?}",
        std::fs::read_dir(dir.join(".smelt/targets/dev/schemas"))
            .map(|rd| rd
                .filter_map(|e| e.ok().map(|e| e.file_name()))
                .collect::<Vec<_>>())
            .unwrap_or_default(),
    );

    // `smelt diff` must include the sub-dir model (not silently skip it) and
    // report no changes after a clean build.
    let diff = run_smelt(&["diff"], dir);
    let stdout = String::from_utf8_lossy(&diff.stdout);
    assert!(
        diff.status.success(),
        "smelt diff reported changes for a clean sub-dir model:\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&diff.stderr),
    );
    assert!(
        stdout.contains("1 model checked"),
        "smelt diff did not check the sub-dir model (expected '1 model checked'), got:\n{stdout}",
    );

    // Drop a column and confirm the generated ALTER statement targets the real
    // db-name table (`main.staging_stg_orders`), not the invalid 3-part canonical
    // form (`main.staging.stg_orders`).
    std::fs::write(
        dir.join("models/staging/stg_orders.sql"),
        "---\nmaterialization: table\n---\n\
         SELECT order_id, customer_id FROM smelt.raw_orders\n",
    )
    .unwrap();
    let diff2 = run_smelt(&["diff"], dir);
    let stdout2 = String::from_utf8_lossy(&diff2.stdout);
    assert!(
        stdout2.contains("main.staging_stg_orders"),
        "expected ALTER on the db-name table 'main.staging_stg_orders', got:\n{stdout2}",
    );
    assert!(
        !stdout2.contains("main.staging.stg_orders"),
        "diff emitted an invalid 3-part table name 'main.staging.stg_orders':\n{stdout2}",
    );
}

/// Phase 3: `smelt build --select` with a non-existent model name must fail
/// with a non-zero exit code and a diagnostic message.
///
/// Since Phase 4 canonical addressing: bare-leaf `--select` args that don't
/// exist in the workspace produce a resolution error (exit non-zero). The
/// previous "exit 0 with 'no models matched'" behavior was replaced by strict
/// argument resolution per `docs/specs/cli.md` §"Argument resolution algorithm".
#[test]
fn no_match_select_emits_stderr_message() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_workspace(dir);

    // First build to seed the project
    let build = run_smelt(&["build"], dir);
    assert!(build.status.success());

    // Run with a selector that matches nothing — now errors per strict resolution.
    let out = run_smelt(&["build", "--select", "nonexistent_model_xyz"], dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "smelt build with non-existent --select should exit non-zero (strict resolution), got exit 0\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("nonexistent_model_xyz") || stderr.contains("not found"),
        "expected model name or 'not found' in stderr, got: {stderr}"
    );
}
