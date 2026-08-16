#![cfg(feature = "duckdb")]
//! `smelt migrate` (plan-only): derives and prints the per-group
//! verdict/technique plan a definition change would take, executing
//! nothing (`docs/specs/definition_deltas.md` §Overview,
//! `docs/outcomes/20260816-definition-delta-migrate-v2/phases/01-plan.md`).

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

const SMELT_YML: &str = "\
name: migrate_fixture
version: 1

paths:
  - models

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

default_materialization: table

state:
  mode: intervals
";

const ORDERS_SQL_V1: &str = "\
SELECT id, amount FROM (VALUES (1, 100.0), (2, 200.0)) AS t(id, amount)
";

/// Stage a minimal one-model project in a fresh temp dir.
fn stage_fixture() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let project_dir = tmp.path().join("migrate_fixture");
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    std::fs::write(project_dir.join("smelt.yml"), SMELT_YML).unwrap();
    std::fs::write(project_dir.join("models/orders.sql"), ORDERS_SQL_V1).unwrap();
    (tmp, project_dir)
}

fn run_build(project_dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"))
}

fn run_migrate(project_dir: &Path, model: &str) -> std::process::Output {
    run_migrate_args(project_dir, model, &[])
}

fn run_migrate_args(project_dir: &Path, model: &str, extra_args: &[&str]) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("migrate")
        .arg(model)
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .args(extra_args)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt migrate`: {e}"))
}

fn set_orders_definition(project_dir: &Path, sql: &str) {
    std::fs::write(project_dir.join("models/orders.sql"), sql).unwrap();
}

const ORDERS_SQL_V2_ADDED_COLUMN: &str = "\
SELECT id, amount, amount * 0.9 AS net_amount FROM (VALUES (1, 100.0), (2, 200.0)) AS t(id, amount)
";

fn table_row_snapshot(project_dir: &Path, table_name: &str) -> Vec<(i64, String)> {
    let db_path = project_dir.join("target/dev.duckdb");
    let conn = duckdb::Connection::open(&db_path).expect("open deployed database");
    let query = format!("SELECT * FROM {table_name} ORDER BY 1");
    let mut stmt = conn.prepare(&query).unwrap();
    stmt.query_map([], |row| {
        let id: i64 = row.get(0)?;
        // Read every remaining column as text so the snapshot captures the
        // full row shape without hard-coding a column count.
        let mut rest = String::new();
        for i in 1.. {
            match row.get::<usize, String>(i) {
                Ok(v) => rest.push_str(&v),
                Err(_) => break,
            }
        }
        Ok((id, rest))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

#[test]
fn migrate_prints_backfill_in_place_plan_for_added_derived_column() {
    let (_tmp, project_dir) = stage_fixture();

    let build_output = run_build(&project_dir);
    assert!(
        build_output.status.success(),
        "initial build failed: stderr={}",
        String::from_utf8_lossy(&build_output.stderr)
    );

    let before_snapshot = table_row_snapshot(&project_dir, "orders");

    // Add a self-derived column — a pure function of an already-stored
    // column, admitting B1 (`Technique::SelfDerivedColumnAdd`).
    std::fs::write(
        project_dir.join("models/orders.sql"),
        "SELECT id, amount, amount * 0.9 AS net_amount FROM (VALUES (1, 100.0), (2, 200.0)) AS \
         t(id, amount)\n",
    )
    .unwrap();

    let output = run_migrate(&project_dir, "orders");
    assert_eq!(
        output.status.code(),
        Some(3),
        "a non-eclipsed derived plan is pending and unapproved, exit 3: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("added column 'net_amount'"),
        "expected the added-column group label in output:\n{stdout}"
    );
    assert!(
        stdout.contains("backfill in place"),
        "expected the BackfillInPlace verdict in output:\n{stdout}"
    );
    assert!(
        stdout.contains("SelfDerivedColumnAdd"),
        "expected the SelfDerivedColumnAdd technique candidate in output:\n{stdout}"
    );
    assert!(
        stdout.contains("full-refresh baseline"),
        "expected the always-present full-refresh baseline line in output:\n{stdout}"
    );

    // Plan-only: nothing executed, the deployed table is unchanged.
    let after_snapshot = table_row_snapshot(&project_dir, "orders");
    assert_eq!(
        before_snapshot, after_snapshot,
        "smelt migrate must not mutate the deployed table"
    );
}

#[test]
fn migrate_presents_rebuild_for_skeleton_change() {
    let (_tmp, project_dir) = stage_fixture();

    let build_output = run_build(&project_dir);
    assert!(
        build_output.status.success(),
        "initial build failed: stderr={}",
        String::from_utf8_lossy(&build_output.stderr)
    );

    let before_snapshot = table_row_snapshot(&project_dir, "orders");

    // Change the GROUP BY grain — a skeleton change (G1), never targetable.
    std::fs::write(
        project_dir.join("models/orders.sql"),
        "SELECT id, SUM(amount) AS amount FROM (VALUES (1, 100.0), (2, 200.0)) AS t(id, amount) \
         GROUP BY id\n",
    )
    .unwrap();

    let output = run_migrate(&project_dir, "orders");
    assert_eq!(
        output.status.code(),
        Some(3),
        "a non-eclipsed derived plan is pending and unapproved, exit 3: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("skeleton change"),
        "expected the SkeletonChange verdict in output:\n{stdout}"
    );
    assert!(
        stdout.contains("full-refresh baseline"),
        "expected the always-present full-refresh baseline line in output:\n{stdout}"
    );

    let after_snapshot = table_row_snapshot(&project_dir, "orders");
    assert_eq!(
        before_snapshot, after_snapshot,
        "smelt migrate must not mutate the deployed table"
    );
}

#[test]
fn migrate_on_unchanged_model_reports_eclipsed() {
    let (_tmp, project_dir) = stage_fixture();

    let build_output = run_build(&project_dir);
    assert!(
        build_output.status.success(),
        "initial build failed: stderr={}",
        String::from_utf8_lossy(&build_output.stderr)
    );

    let output = run_migrate(&project_dir, "orders");
    assert!(
        output.status.success(),
        "migrate failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("eclipsed: nothing to do"),
        "expected the eclipsed short-circuit in output:\n{stdout}"
    );
}

#[test]
fn eclipsed_delta_exits_zero() {
    let (_tmp, project_dir) = stage_fixture();
    let build_output = run_build(&project_dir);
    assert!(build_output.status.success());

    let output = run_migrate(&project_dir, "orders");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn plan_step_records_hash_and_exits_three() {
    let (_tmp, project_dir) = stage_fixture();
    let build_output = run_build(&project_dir);
    assert!(build_output.status.success());

    set_orders_definition(&project_dir, ORDERS_SQL_V2_ADDED_COLUMN);

    let output = run_migrate(&project_dir, "orders");
    assert_eq!(
        output.status.code(),
        Some(3),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("plan hash:"),
        "expected the printed plan hash in output:\n{stdout}"
    );

    let approvals_path = project_dir.join(".smelt/targets/dev/migration-approvals.json");
    assert!(
        approvals_path.exists(),
        "expected the plan step to record an approval-store file"
    );
    let content = std::fs::read_to_string(&approvals_path).unwrap();
    assert!(
        content.contains("orders"),
        "expected the recorded approval to be keyed by the model: {content}"
    );
}

#[test]
fn json_flag_emits_plan_hash_and_verdicts() {
    let (_tmp, project_dir) = stage_fixture();
    let build_output = run_build(&project_dir);
    assert!(build_output.status.success());

    set_orders_definition(&project_dir, ORDERS_SQL_V2_ADDED_COLUMN);

    let output = run_migrate_args(&project_dir, "orders", &["--json"]);
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("not valid JSON: {e}\n{stdout}"));

    assert_eq!(json["model"], "orders");
    assert_eq!(json["eclipsed"], false);
    assert!(json["plan_hash"].as_str().unwrap().starts_with("sha256:"));
    assert!(json["approved"].is_boolean());
    let groups = json["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["verdict"], "backfill_in_place");
    assert_eq!(
        groups[0]["candidates"][0]["technique"],
        "SelfDerivedColumnAdd"
    );
}

#[test]
fn apply_without_recorded_plan_refuses() {
    let (_tmp, project_dir) = stage_fixture();
    let build_output = run_build(&project_dir);
    assert!(build_output.status.success());

    set_orders_definition(&project_dir, ORDERS_SQL_V2_ADDED_COLUMN);
    let before_snapshot = table_row_snapshot(&project_dir, "orders");

    let output = run_migrate_args(&project_dir, "orders", &["--apply"]);
    assert_eq!(
        output.status.code(),
        Some(3),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("plan hash:"),
        "expected --apply to print the fresh plan on refusal:\n{stdout}"
    );

    let approvals_path = project_dir.join(".smelt/targets/dev/migration-approvals.json");
    assert!(
        approvals_path.exists(),
        "--apply should still record the freshly derived plan it refused"
    );

    let after_snapshot = table_row_snapshot(&project_dir, "orders");
    assert_eq!(
        before_snapshot, after_snapshot,
        "a refused --apply must not execute anything"
    );
}

#[test]
fn apply_refuses_when_definition_changed_since_plan() {
    let (_tmp, project_dir) = stage_fixture();
    let build_output = run_build(&project_dir);
    assert!(build_output.status.success());

    set_orders_definition(&project_dir, ORDERS_SQL_V2_ADDED_COLUMN);
    let plan_output = run_migrate(&project_dir, "orders");
    assert_eq!(plan_output.status.code(), Some(3));

    // Edit again after the plan step recorded its hash.
    set_orders_definition(
        &project_dir,
        "SELECT id, amount, amount * 0.5 AS net_amount FROM (VALUES (1, 100.0), (2, 200.0)) AS \
         t(id, amount)\n",
    );

    let apply_output = run_migrate_args(&project_dir, "orders", &["--apply"]);
    assert_eq!(
        apply_output.status.code(),
        Some(3),
        "stderr={}",
        String::from_utf8_lossy(&apply_output.stderr)
    );
    let stderr = String::from_utf8_lossy(&apply_output.stderr);
    assert!(
        stderr.contains("no approved plan matching"),
        "expected the hash-mismatch refusal message:\n{stderr}"
    );
}

#[test]
fn apply_with_matching_hash_is_accepted() {
    let (_tmp, project_dir) = stage_fixture();
    let build_output = run_build(&project_dir);
    assert!(build_output.status.success());

    set_orders_definition(&project_dir, ORDERS_SQL_V2_ADDED_COLUMN);
    let plan_output = run_migrate(&project_dir, "orders");
    assert_eq!(plan_output.status.code(), Some(3));
    let before_snapshot = table_row_snapshot(&project_dir, "orders");

    let apply_output = run_migrate_args(&project_dir, "orders", &["--apply"]);
    assert_eq!(
        apply_output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&apply_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&apply_output.stdout);
    assert!(
        stdout.contains("approved"),
        "expected the approved-and-not-yet-executed message:\n{stdout}"
    );

    let after_snapshot = table_row_snapshot(&project_dir, "orders");
    assert_eq!(
        before_snapshot, after_snapshot,
        "execution has not landed yet — --apply must not mutate the table this phase"
    );
}
