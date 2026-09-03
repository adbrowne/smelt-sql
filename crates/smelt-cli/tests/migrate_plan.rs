#![cfg(feature = "duckdb")]
//! Integration tests for `smelt migrate` — the plan-only definition-delta
//! migration verb (`docs/outcomes/20260815-definition-delta-migrate/phases/
//! 02-plan.md`; `docs/specs/definition_deltas.md` §"`smelt migrate`").
//!
//! Real-binary subprocess harness, matching `tests/list_clean.rs`: scaffold
//! a project, `smelt build` once (recording a `DeployedSchema` with
//! `model_sql`), edit the model, then invoke `smelt migrate` and assert on
//! its output. `DUCKDB_LIB_DIR` must be set — every DuckDB-backed test in
//! this crate skips loudly when it is not.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

fn skip_without_duckdb_lib() -> bool {
    if std::env::var("DUCKDB_LIB_DIR").is_err() {
        eprintln!("skipping: DUCKDB_LIB_DIR not set (migrate tests require DuckDB)");
        return true;
    }
    false
}

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn run(project_dir: &Path, args: &[&str]) -> Output {
    Command::new(smelt_bin())
        .args(args)
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt {args:?}`: {e}"))
}

const MODEL_V1: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n";

const MODEL_V2_SELF_DERIVED: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount, amount - discount AS net_amount FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n";

/// Scaffold a minimal project via `smelt init`, then replace the default
/// model with a self-contained one (no seeds/sources needed) so `smelt
/// build` records a real `DeployedSchema` for it.
fn scaffold(tmp: &TempDir) -> PathBuf {
    let project_dir = tmp.path().join("proj");
    let init_out = Command::new(smelt_bin())
        .arg("init")
        .arg(&project_dir)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt init`: {e}"));
    assert!(
        init_out.status.success(),
        "smelt init should succeed.\nstderr: {}",
        String::from_utf8_lossy(&init_out.stderr)
    );

    // Remove the scaffolded models so only our own fixture model exists —
    // keeps `smelt migrate`'s output scoped to the model under test.
    let models_dir = project_dir.join("models");
    for entry in std::fs::read_dir(&models_dir).expect("read models dir") {
        let entry = entry.expect("dir entry");
        if entry.path().extension().and_then(|e| e.to_str()) == Some("sql") {
            std::fs::remove_file(entry.path()).expect("remove scaffolded model");
        }
    }

    std::fs::write(models_dir.join("net_orders.sql"), MODEL_V1).expect("write model v1");

    project_dir
}

fn build(project_dir: &Path) {
    let out = run(project_dir, &["build"]);
    assert!(
        out.status.success(),
        "smelt build should succeed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn edit_model_to_add_net_amount(project_dir: &Path) {
    std::fs::write(
        project_dir.join("models").join("net_orders.sql"),
        MODEL_V2_SELF_DERIVED,
    )
    .expect("write model v2");
}

#[test]
fn migrate_prints_per_group_verdict_and_technique() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    edit_model_to_add_net_amount(&project_dir);

    let out = run(&project_dir, &["migrate", "net_orders"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // A newly-derived, non-eclipsed plan is unapproved on its first
    // printing — exit 3 (`docs/specs/cli.md` §"Exit codes").
    assert_eq!(
        out.status.code(),
        Some(3),
        "smelt migrate should exit 3 for an unapproved plan.\nstdout: {stdout}\nstderr: {stderr}"
    );

    assert!(
        stdout.contains("net_amount"),
        "expected the net_amount column group in output:\n{stdout}"
    );
    assert!(
        stdout.to_lowercase().contains("backfill in place"),
        "expected a backfill-in-place verdict:\n{stdout}"
    );
    assert!(
        stdout.contains("SelfDerivedColumnAdd"),
        "expected the SelfDerivedColumnAdd technique named:\n{stdout}"
    );
    assert!(
        stdout.contains("plan hash:"),
        "expected a plan hash line:\n{stdout}"
    );
}

#[test]
fn migrate_on_unchanged_definition_prints_nothing_to_do() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    // No edit — the definition is unchanged.

    let out = run(&project_dir, &["migrate", "net_orders"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "smelt migrate should exit 0 on an unchanged definition.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("eclipsed") && stdout.to_lowercase().contains("nothing to do"),
        "expected an eclipsed/nothing-to-do message:\n{stdout}"
    );
}

#[test]
fn migrate_without_a_recorded_definition_refuses_loudly() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    // No `smelt build` — the model has never been deployed under schema
    // tracking on this target.

    let out = run(&project_dir, &["migrate", "net_orders"]);
    assert!(
        !out.status.success(),
        "smelt migrate must refuse (non-zero exit) for a never-built model"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("no recorded definition"),
        "expected a refusal naming the missing recorded definition:\n{stderr}"
    );
}

#[test]
fn migrate_executes_nothing() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    edit_model_to_add_net_amount(&project_dir);

    let db_path = project_dir.join("dev.duckdb");
    let (schema_before, rows_before) = snapshot_table(&db_path);

    let out = run(&project_dir, &["migrate", "net_orders"]);
    // An unapproved non-eclipsed plan exits 3, not 0 — see
    // `migrate_prints_per_group_verdict_and_technique`.
    assert_eq!(
        out.status.code(),
        Some(3),
        "smelt migrate should exit 3.\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let (schema_after, rows_after) = snapshot_table(&db_path);
    assert_eq!(
        schema_before, schema_after,
        "smelt migrate must not alter the stored table's schema"
    );
    assert_eq!(
        rows_before, rows_after,
        "smelt migrate must not alter the stored table's contents"
    );
}

/// `(column_name, column_type)` schema plus row count for `main.net_orders`.
fn snapshot_table(db_path: &Path) -> (Vec<(String, String)>, i64) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let mut schema_stmt = conn
        .prepare(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = 'main' AND table_name = 'net_orders' \
             ORDER BY ordinal_position",
        )
        .expect("prepare schema query");
    let schema: Vec<(String, String)> = schema_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query schema")
        .collect::<Result<_, _>>()
        .expect("collect schema rows");

    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM main.net_orders", [], |row| row.get(0))
        .expect("query row count");

    (schema, row_count)
}
