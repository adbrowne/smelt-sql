#![cfg(feature = "duckdb")]
//! Integration tests for `smelt migrate --apply` / `--json` — the
//! approval-store + execution + CI-mode phase of the definition-delta
//! migration verb (`docs/outcomes/20260815-definition-delta-migrate/phases/
//! 03-plan.md`; `docs/specs/definition_deltas.md` §"`smelt migrate`").
//!
//! Real-binary subprocess harness, matching `tests/migrate_plan.rs`.
//! `DUCKDB_LIB_DIR` must be set — every DuckDB-backed test in this crate
//! skips loudly when it is not.

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

// Formatting-only edit — an eclipsed delta, never a pending migration.
const MODEL_V1_REFORMATTED: &str = "---\nmaterialization: table\n---\n\
SELECT\n  id,\n  amount,\n  discount\nFROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n";

// Redefines the existing `discount` column as a function of the unchanged
// `amount` column — a D1 stored-derivable expression change
// (`Technique::SelfDerivedColumnRewrite`, a plain `UPDATE` with no `ALTER`)
// and therefore `rerun_safe: true`, unlike `MODEL_V2_SELF_DERIVED`'s
// `SelfDerivedColumnAdd`.
const MODEL_V4_RERUN_SAFE_REWRITE: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, amount * 0.1 AS discount FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n";

/// A grain-changing edit — no admissible in-place technique (skeleton
/// change).
const MODEL_V3_SKELETON_CHANGE: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount, count(*) AS n FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount) GROUP BY id, amount, discount\n";

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
    // `smelt init`'s scaffold declares no `state:` key, so it defaults to
    // `state.mode: stateless` (`docs/specs/state.md` §"`state.mode` and
    // what each posture provides"); `smelt migrate`'s schema-tracking
    // lookups here need a posture that actually records a deployed schema.
    let smelt_yml_path = project_dir.join("smelt.yml");
    let mut smelt_yml = std::fs::read_to_string(&smelt_yml_path).unwrap();
    smelt_yml.push_str("state:\n  mode: intervals\n");
    std::fs::write(&smelt_yml_path, smelt_yml).unwrap();

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

fn write_model(project_dir: &Path, content: &str) {
    std::fs::write(project_dir.join("models").join("net_orders.sql"), content)
        .expect("write model");
}

fn approvals_path(project_dir: &Path) -> PathBuf {
    project_dir
        .join(".smelt")
        .join("targets")
        .join("dev")
        .join("migration_approvals.json")
}

/// `(column_name, column_type)` schema for `main.net_orders`.
fn snapshot_table(db_path: &Path) -> Vec<(String, String)> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let mut schema_stmt = conn
        .prepare(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = 'main' AND table_name = 'net_orders' \
             ORDER BY ordinal_position",
        )
        .expect("prepare schema query");
    schema_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query schema")
        .collect::<Result<_, _>>()
        .expect("collect schema rows")
}

#[test]
fn plan_then_apply_executes_and_clears_the_delta() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    write_model(&project_dir, MODEL_V2_SELF_DERIVED);

    let plan_out = run(&project_dir, &["migrate", "net_orders"]);
    assert_eq!(
        plan_out.status.code(),
        Some(3),
        "plan step should exit 3 for an unapproved plan.\nstderr: {}",
        String::from_utf8_lossy(&plan_out.stderr)
    );

    let apply_out = run(&project_dir, &["migrate", "net_orders", "--apply"]);
    assert!(
        apply_out.status.success(),
        "smelt migrate --apply should exit 0.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply_out.stdout),
        String::from_utf8_lossy(&apply_out.stderr)
    );

    let db_path = project_dir.join("dev.duckdb");
    let schema = snapshot_table(&db_path);
    assert!(
        schema.iter().any(|(name, _)| name == "net_amount"),
        "expected net_amount to exist after apply: {schema:?}"
    );

    let followup = run(&project_dir, &["migrate", "net_orders"]);
    let followup_stdout = String::from_utf8_lossy(&followup.stdout);
    assert!(
        followup.status.success(),
        "a following smelt migrate should report eclipsed (exit 0).\nstdout: {followup_stdout}"
    );
    assert!(
        followup_stdout.contains("eclipsed"),
        "expected an eclipsed report after apply cleared the delta:\n{followup_stdout}"
    );
}

#[test]
fn apply_without_a_prior_plan_refuses() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    write_model(&project_dir, MODEL_V2_SELF_DERIVED);

    let out = run(&project_dir, &["migrate", "net_orders", "--apply"]);
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("migrate net_orders")
            || stderr.to_lowercase().contains("no approved plan"),
        "expected a refusal naming the plan step:\n{stderr}"
    );
}

#[test]
fn apply_after_the_model_changed_refuses_and_reprints() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    write_model(&project_dir, MODEL_V2_SELF_DERIVED);

    let plan_out = run(&project_dir, &["migrate", "net_orders"]);
    assert_eq!(plan_out.status.code(), Some(3));

    // Edit the SQL again between plan and apply.
    write_model(
        &project_dir,
        "---\nmaterialization: table\n---\n\
SELECT id, amount, discount, amount + discount AS net_amount FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n",
    );

    let apply_out = run(&project_dir, &["migrate", "net_orders", "--apply"]);
    assert_eq!(apply_out.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&apply_out.stdout);
    assert!(
        stdout.contains("net_amount"),
        "expected the freshly re-derived plan to be printed:\n{stdout}"
    );

    // Nothing was executed — the stored table is unchanged.
    let db_path = project_dir.join("dev.duckdb");
    let schema = snapshot_table(&db_path);
    assert!(
        !schema.iter().any(|(name, _)| name == "net_amount"),
        "apply must not have executed anything: {schema:?}"
    );
}

#[test]
fn apply_on_a_skeleton_change_refuses() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    write_model(&project_dir, MODEL_V3_SKELETON_CHANGE);

    let plan_out = run(&project_dir, &["migrate", "net_orders"]);
    assert_eq!(plan_out.status.code(), Some(3));

    let apply_out = run(&project_dir, &["migrate", "net_orders", "--apply"]);
    assert_eq!(
        apply_out.status.code(),
        Some(1),
        "a skeleton change must refuse apply with exit 1.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply_out.stdout),
        String::from_utf8_lossy(&apply_out.stderr)
    );
    let stderr = String::from_utf8_lossy(&apply_out.stderr);
    assert!(
        stderr.to_lowercase().contains("full refresh")
            || stderr.to_lowercase().contains("full-refresh"),
        "expected the refusal to point at a full refresh:\n{stderr}"
    );
}

#[test]
fn json_eclipsed_exits_zero() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    write_model(&project_dir, MODEL_V1_REFORMATTED);

    let out = run(&project_dir, &["migrate", "net_orders", "--json"]);
    assert!(
        out.status.success(),
        "an eclipsed delta should exit 0.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(json["verdict"], "eclipsed");
}

#[test]
fn json_pending_migration_exits_three() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    write_model(&project_dir, MODEL_V2_SELF_DERIVED);

    let out = run(&project_dir, &["migrate", "net_orders", "--json"]);
    assert_eq!(out.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(json["approved"], false);
    assert!(json["plan_hash"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(json["groups"].as_array().is_some_and(|g| !g.is_empty()));
}

#[test]
fn json_after_approval_exits_zero() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    write_model(&project_dir, MODEL_V2_SELF_DERIVED);

    // First invocation (plan step, human path) records the approval.
    let first = run(&project_dir, &["migrate", "net_orders"]);
    assert_eq!(first.status.code(), Some(3));

    // Second invocation sees the same plan already on record.
    let out = run(&project_dir, &["migrate", "net_orders", "--json"]);
    assert!(
        out.status.success(),
        "an already-approved plan should exit 0.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(json["approved"], true);
}

#[test]
fn interrupted_non_rerun_safe_apply_refuses_on_reinvocation() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    // `SelfDerivedColumnAdd` (ALTER ADD COLUMN + UPDATE) is not rerun-safe —
    // a second ALTER ADD COLUMN on the same column fails.
    write_model(&project_dir, MODEL_V2_SELF_DERIVED);

    let plan_out = run(&project_dir, &["migrate", "net_orders"]);
    assert_eq!(plan_out.status.code(), Some(3));

    // Simulate an interrupted apply: flip the recorded approval's
    // `in_progress` flag without actually running anything.
    let path = approvals_path(&project_dir);
    let content = std::fs::read_to_string(&path).expect("read approvals file");
    let mut json: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
    for (_, entry) in json.as_object_mut().expect("object").iter_mut() {
        entry["in_progress"] = serde_json::Value::Bool(true);
    }
    std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap())
        .expect("write approvals file");

    let apply_out = run(&project_dir, &["migrate", "net_orders", "--apply"]);
    assert_eq!(
        apply_out.status.code(),
        Some(1),
        "an interrupted, non-rerun-safe apply must refuse with exit 1.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply_out.stdout),
        String::from_utf8_lossy(&apply_out.stderr)
    );
    let stderr = String::from_utf8_lossy(&apply_out.stderr);
    assert!(
        stderr.to_lowercase().contains("full refresh")
            || stderr.to_lowercase().contains("full-refresh"),
        "expected the refusal to point at a full refresh:\n{stderr}"
    );
}

/// The leg the phase-3 summary flagged as untested: an `in_progress`
/// approval whose plan IS `all_rerun_safe()` (unlike the test above) must
/// resume — re-execute the identical script — rather than refuse.
#[test]
fn apply_resumes_rerun_safe_in_progress_plan() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let project_dir = scaffold(&tmp);
    build(&project_dir);
    write_model(&project_dir, MODEL_V4_RERUN_SAFE_REWRITE);

    let plan_out = run(&project_dir, &["migrate", "net_orders"]);
    assert_eq!(plan_out.status.code(), Some(3));

    // Simulate an interrupted apply the same way
    // `interrupted_non_rerun_safe_apply_refuses_on_reinvocation` does.
    let path = approvals_path(&project_dir);
    let content = std::fs::read_to_string(&path).expect("read approvals file");
    let mut json: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
    for (_, entry) in json.as_object_mut().expect("object").iter_mut() {
        entry["in_progress"] = serde_json::Value::Bool(true);
    }
    std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap())
        .expect("write approvals file");

    let apply_out = run(&project_dir, &["migrate", "net_orders", "--apply"]);
    assert!(
        apply_out.status.success(),
        "an interrupted, rerun-safe apply should resume and exit 0.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply_out.stdout),
        String::from_utf8_lossy(&apply_out.stderr)
    );

    let db_path = project_dir.join("dev.duckdb");
    let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
    let discount: f64 = conn
        .query_row(
            "SELECT discount FROM main.net_orders WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("query discount");
    assert!(
        (discount - 10.0).abs() < 1e-9,
        "expected discount to be rewritten to amount * 0.1 = 10.0, got {discount}"
    );
}
