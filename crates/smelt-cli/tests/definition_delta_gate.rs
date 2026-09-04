#![cfg(feature = "duckdb")]
//! `smelt run`'s definition-delta gate (`docs/outcomes/
//! 20260815-definition-delta-migrate/phases/03b-plan.md`;
//! `docs/specs/definition_deltas.md` §"Detection"): a maintained (incremental)
//! model with a pending, non-eclipsed, unapproved definition delta refuses to
//! fold a data delta rather than silently maintaining a table whose
//! definition no longer matches its contents.
//!
//! Real-binary subprocess harness, matching `tests/since_upstream.rs`.
//! `DUCKDB_LIB_DIR` must be set — every DuckDB-backed test in this crate
//! skips loudly when it is not.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use duckdb::Connection;
use tempfile::TempDir;

fn skip_without_duckdb_lib() -> bool {
    if std::env::var("DUCKDB_LIB_DIR").is_err() {
        eprintln!("skipping: DUCKDB_LIB_DIR not set (definition-delta gate tests require DuckDB)");
        return true;
    }
    false
}

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

const SILVER_V1: &str = "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
timeseries:\n  partition_column: event_date\n  event_time_column: event_date\n  granularity: day\n---\n\
SELECT id, d AS event_date, 'v1' AS note FROM smelt.sources.bronze\n";

// Changes an existing column's literal value — a non-eclipsed,
// backfill-in-place definition delta that is NOT a pure column addition,
// so the run gate's `pure_column_addition` carve-out does not apply.
const SILVER_V2_SELF_DERIVED: &str =
    "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
timeseries:\n  partition_column: event_date\n  event_time_column: event_date\n  granularity: day\n---\n\
SELECT id, d AS event_date, 'v2' AS note FROM smelt.sources.bronze\n";

// A non-incremental (plain table) model with no `refresh:`/`grain:` — the
// gate only applies to maintained models.
const PLAIN_V1: &str = "---\nmaterialization: table\n---\n\
SELECT 1 AS x\n";
const PLAIN_V2: &str = "---\nmaterialization: table\n---\n\
SELECT 1 AS x, 2 AS y\n";

fn stage_workspace(parent: &Path) -> PathBuf {
    let root = parent.join("proj");
    write(
        &root,
        "smelt.yml",
        "name: definition_delta_gate_ws\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\nstate:\n  mode: intervals\n",
    );
    write(
        &root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: id\n  type: INTEGER\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(&root, "models/silver.sql", SILVER_V1);
    write(&root, "models/plain.sql", PLAIN_V1);
    std::fs::create_dir_all(root.join("target")).unwrap();
    root
}

fn seed_sources(db_path: &Path) {
    let conn = Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS main;\n\
         CREATE TABLE main.sources_bronze (id INTEGER, d DATE);\n\
         INSERT INTO main.sources_bronze \
           SELECT i, DATE '2026-01-01' + CAST(i - 1 AS INTEGER) FROM range(1, 4) t(i);\n",
    )
    .expect("seed source table");
}

fn run_smelt(project_dir: &Path, args: &[&str]) -> Output {
    Command::new(smelt_bin())
        .arg("run")
        .args(args)
        .arg("--project-dir")
        .arg(project_dir)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

fn run_smelt_generic(project_dir: &Path, args: &[&str]) -> Output {
    Command::new(smelt_bin())
        .args(args)
        .arg("--project-dir")
        .arg(project_dir)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt {args:?}`: {e}"))
}

const EVENT_RANGE: &[&str] = &[
    "--event-time-start",
    "2026-01-01",
    "--event-time-end",
    "2026-01-04",
];

#[test]
fn run_refuses_incremental_fold_over_pending_delta() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = stage_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_sources(&db_path);

    let first = run_smelt(&project_dir, EVENT_RANGE);
    assert!(
        first.status.success(),
        "initial run should succeed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    write(&project_dir, "models/silver.sql", SILVER_V2_SELF_DERIVED);

    let second = run_smelt(&project_dir, EVENT_RANGE);
    assert!(
        !second.status.success(),
        "a run over a pending definition delta must refuse.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("silver"),
        "expected the model named: {stderr}"
    );
    assert!(
        stderr.contains("DefinitionDeltaPending"),
        "expected the diagnostic code named: {stderr}"
    );
    assert!(
        stderr.contains("smelt migrate"),
        "expected the fix hint: {stderr}"
    );
}

#[test]
fn run_refusal_exits_3() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = stage_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_sources(&db_path);

    let first = run_smelt(&project_dir, EVENT_RANGE);
    assert!(first.status.success());

    write(&project_dir, "models/silver.sql", SILVER_V2_SELF_DERIVED);

    let second = run_smelt(&project_dir, EVENT_RANGE);
    assert_eq!(
        second.status.code(),
        Some(3),
        "expected exit 3.\nstderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
}

#[test]
fn full_refresh_run_proceeds_over_pending_delta() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = stage_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_sources(&db_path);

    let first = run_smelt(&project_dir, EVENT_RANGE);
    assert!(first.status.success());

    write(&project_dir, "models/silver.sql", SILVER_V2_SELF_DERIVED);

    let mut args = EVENT_RANGE.to_vec();
    args.push("--full-refresh");
    let second = run_smelt(&project_dir, &args);
    assert!(
        second.status.success(),
        "--full-refresh should proceed over a pending delta.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
}

#[test]
fn run_proceeds_after_migrate_plan_records_approval() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = stage_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_sources(&db_path);

    let first = run_smelt(&project_dir, EVENT_RANGE);
    assert!(first.status.success());

    write(&project_dir, "models/silver.sql", SILVER_V2_SELF_DERIVED);

    let plan_out = run_smelt_generic(&project_dir, &["migrate", "silver"]);
    assert_eq!(
        plan_out.status.code(),
        Some(3),
        "plan step should exit 3 for an unapproved plan.\nstderr: {}",
        String::from_utf8_lossy(&plan_out.stderr)
    );

    let second = run_smelt(&project_dir, EVENT_RANGE);
    assert!(
        second.status.success(),
        "a run should fold normally once the plan is approved (recorded by the plan step).\n\
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
}

#[test]
fn non_incremental_model_never_refuses() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = stage_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_sources(&db_path);

    let first = run_smelt(&project_dir, &["--select", "plain"]);
    assert!(
        first.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    write(&project_dir, "models/plain.sql", PLAIN_V2);

    let second = run_smelt(&project_dir, &["--select", "plain"]);
    assert!(
        second.status.success(),
        "a table/view model's changed definition must never be gated.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
}
