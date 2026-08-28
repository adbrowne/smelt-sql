#![cfg(feature = "duckdb")]
//! `smelt explain <model>` reports a pending definition delta ahead of a
//! run (`docs/outcomes/20260815-definition-delta-migrate/phases/
//! 03b-plan.md`; `docs/specs/definition_deltas.md` §"Detection"), without
//! deriving or executing anything beyond the plan derivation itself.
//!
//! Real-binary subprocess harness, matching `tests/definition_delta_gate.rs`.
//! `DUCKDB_LIB_DIR` must be set — every DuckDB-backed test in this crate
//! skips loudly when it is not.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use duckdb::Connection;
use tempfile::TempDir;

fn skip_without_duckdb_lib() -> bool {
    if std::env::var("DUCKDB_LIB_DIR").is_err() {
        eprintln!(
            "skipping: DUCKDB_LIB_DIR not set (explain definition-delta tests require DuckDB)"
        );
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

const SILVER_V2_SELF_DERIVED: &str =
    "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
timeseries:\n  partition_column: event_date\n  event_time_column: event_date\n  granularity: day\n---\n\
SELECT id, d AS event_date, 'v2' AS note FROM smelt.sources.bronze\n";

fn stage_workspace(parent: &Path) -> PathBuf {
    let root = parent.join("proj");
    write(
        &root,
        "smelt.yml",
        "name: explain_definition_delta_ws\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n",
    );
    write(
        &root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: id\n  type: INTEGER\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(&root, "models/silver.sql", SILVER_V1);
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
        .args(args)
        .arg("--project-dir")
        .arg(project_dir)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt {args:?}`: {e}"))
}

const EVENT_RANGE: &[&str] = &[
    "run",
    "--event-time-start",
    "2026-01-01",
    "--event-time-end",
    "2026-01-04",
];

#[test]
fn explain_reports_pending_definition_delta() {
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

    let out = run_smelt(&project_dir, &["explain", "silver"]);
    assert!(
        out.status.success(),
        "smelt explain should succeed regardless of the pending delta.\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Definition delta") && stdout.to_uppercase().contains("PENDING"),
        "expected a pending definition-delta line: {stdout}"
    );
}

#[test]
fn explain_json_carries_definition_delta() {
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

    let out = run_smelt(&project_dir, &["explain", "silver", "--json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let delta = &json["definition_delta"];
    assert_eq!(delta["status"], "pending");
    assert!(
        delta["verdict"].is_string() && !delta["verdict"].as_str().unwrap().is_empty(),
        "expected a named verdict: {delta}"
    );
    assert!(
        delta["plan_hash"].is_string() && !delta["plan_hash"].as_str().unwrap().is_empty(),
        "expected a plan hash: {delta}"
    );
}

#[test]
fn explain_omits_definition_delta_when_none() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = stage_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_sources(&db_path);

    let first = run_smelt(&project_dir, EVENT_RANGE);
    assert!(first.status.success());

    // No edit — the recorded and current definitions are identical.
    let out = run_smelt(&project_dir, &["explain", "silver", "--json"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(
        json.get("definition_delta").is_none(),
        "expected no definition_delta field: {stdout}"
    );

    let text_out = run_smelt(&project_dir, &["explain", "silver"]);
    assert!(text_out.status.success());
    let text_stdout = String::from_utf8_lossy(&text_out.stdout);
    assert!(
        !text_stdout.contains("Definition delta"),
        "expected no definition-delta line: {text_stdout}"
    );
}
