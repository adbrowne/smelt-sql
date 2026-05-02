//! Phase 2 (TB-B + TB-C) — Declared return type flows into model schema.
//!
//! Tests verify that:
//!   1. `-> Expr<Double>` annotation on a `smelt.define` function produces
//!      `DataType::Double` for a column projecting that call in the model schema.
//!   2. `-> Expr<Boolean>` annotation produces `DataType::Boolean`.
//!   3. `SUM(smelt.functions.safe_revenue(amount))` infers as `Double`, not
//!      `BigInt`, once the call is typed correctly (TB-C fix).
//!   4. A Tier 1/2 function without `-> <Type>` still produces `Unknown` (no
//!      regression).

use std::path::PathBuf;

use smelt_db::{typed_model_schema, Database, SourceFile, Workspace};
use smelt_types::DataType;

// ─── helpers ──────────────────────────────────────────────────────────────────

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

// ─── Test 1: Double return type appears in model schema ───────────────────────

/// `safe_revenue(a: Expr<Double>) -> Expr<Double>` — the staging model's
/// `amount` column must have type `Double`, not `Unknown`.
#[test]
fn test_double_return_type_appears_in_model_schema() {
    let root = PathBuf::from("/fake/project/tb_b_double");
    let fn_path = root.join("functions").join("revenue.sql");
    let fn_src =
        "smelt.define safe_revenue(a: Expr<Double>) -> Expr<Double> AS (COALESCE(a, 0.0))\n";

    // Upstream model that has an `amount` column so the function call is valid.
    let upstream_path = root.join("models").join("orders.sql");
    let upstream_src = "SELECT 42.0 AS amount\n";

    let staging_path = root.join("models").join("staging.sql");
    let staging_src =
        "SELECT smelt.functions.safe_revenue(amount) AS amount FROM smelt.models.orders\n";

    let (db, ws, files) = build_db(
        root,
        &[
            (fn_path, fn_src),
            (upstream_path, upstream_src),
            (staging_path.clone(), staging_src),
        ],
    );
    let staging_file = files[2];

    let schema = typed_model_schema(&db, ws, staging_file);

    let col = schema
        .columns
        .iter()
        .find(|c| c.name == "amount")
        .expect("expected `amount` column in schema");

    let typed = col
        .data_type
        .as_ref()
        .expect("expected `amount` to have an inferred type");

    assert_eq!(
        typed.data_type,
        DataType::Double,
        "expected `amount` to be Double (declared return type), got {:?}",
        typed.data_type,
    );
}

// ─── Test 2: Boolean return type appears in model schema ──────────────────────

/// `is_shipped(s: Expr<Text>) -> Expr<Boolean>` — the model's `is_shipped`
/// column must have type `Boolean`, not `Unknown`.
#[test]
fn test_boolean_return_type_appears_in_model_schema() {
    let root = PathBuf::from("/fake/project/tb_b_boolean");
    let fn_path = root.join("functions").join("shipping.sql");
    let fn_src = "smelt.define is_shipped(s: Expr<Text>) -> Expr<Boolean> AS (s = 'shipped')\n";

    let upstream_path = root.join("models").join("orders.sql");
    let upstream_src = "SELECT 'shipped' AS status\n";

    let model_path = root.join("models").join("classified.sql");
    let model_src =
        "SELECT smelt.functions.is_shipped(status) AS is_shipped FROM smelt.models.orders\n";

    let (db, ws, files) = build_db(
        root,
        &[
            (fn_path, fn_src),
            (upstream_path, upstream_src),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[2];

    let schema = typed_model_schema(&db, ws, model_file);

    let col = schema
        .columns
        .iter()
        .find(|c| c.name == "is_shipped")
        .expect("expected `is_shipped` column in schema");

    let typed = col
        .data_type
        .as_ref()
        .expect("expected `is_shipped` to have an inferred type");

    assert_eq!(
        typed.data_type,
        DataType::Boolean,
        "expected `is_shipped` to be Boolean (declared return type), got {:?}",
        typed.data_type,
    );
}

// ─── Test 3: SUM over function-call typed as Double infers Double ─────────────

/// `SUM(smelt.functions.safe_revenue(amount))` must infer as `Double` (TB-C).
/// Previously, when the inner call returned `Unknown`, SUM fell back to `BigInt`.
#[test]
fn test_sum_over_function_double_infers_double() {
    let root = PathBuf::from("/fake/project/tb_c_sum");
    let fn_path = root.join("functions").join("revenue.sql");
    let fn_src =
        "smelt.define safe_revenue(a: Expr<Double>) -> Expr<Double> AS (COALESCE(a, 0.0))\n";

    let upstream_path = root.join("models").join("orders.sql");
    let upstream_src = "SELECT 42.0 AS amount\n";

    let agg_path = root.join("models").join("revenue_summary.sql");
    let agg_src = "SELECT SUM(smelt.functions.safe_revenue(amount)) AS total_revenue FROM smelt.models.orders\n";

    let (db, ws, files) = build_db(
        root,
        &[
            (fn_path, fn_src),
            (upstream_path, upstream_src),
            (agg_path.clone(), agg_src),
        ],
    );
    let agg_file = files[2];

    let schema = typed_model_schema(&db, ws, agg_file);

    let col = schema
        .columns
        .iter()
        .find(|c| c.name == "total_revenue")
        .expect("expected `total_revenue` column in schema");

    let typed = col
        .data_type
        .as_ref()
        .expect("expected `total_revenue` to have an inferred type");

    assert_eq!(
        typed.data_type,
        DataType::Double,
        "expected SUM(safe_revenue(amount)) to be Double, got {:?} — TB-C: SUM(Double) must return Double",
        typed.data_type,
    );
}

// ─── Test 4: Untyped function (Tier 1/2) produces Unknown ─────────────────────

/// `smelt.define f(x) AS (x + 1)` — no return type declared.
/// Model projecting the call must see `Unknown` for that column.
#[test]
fn test_untyped_function_produces_unknown() {
    let root = PathBuf::from("/fake/project/tb_b_untyped");
    let fn_path = root.join("functions").join("add_one.sql");
    let fn_src = "smelt.define f(x) AS (x + 1)\n";

    let upstream_path = root.join("models").join("src.sql");
    let upstream_src = "SELECT 1 AS x\n";

    let model_path = root.join("models").join("derived.sql");
    let model_src = "SELECT smelt.functions.f(x) AS result FROM smelt.models.src\n";

    let (db, ws, files) = build_db(
        root,
        &[
            (fn_path, fn_src),
            (upstream_path, upstream_src),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[2];

    let schema = typed_model_schema(&db, ws, model_file);

    let col = schema
        .columns
        .iter()
        .find(|c| c.name == "result")
        .expect("expected `result` column in schema");

    // Untyped function: type is None or Unknown — both are acceptable.
    // The column may have no data_type at all, or an explicit Unknown.
    let actual_type = col.data_type.as_ref().map(|t| &t.data_type);
    let is_unknown = matches!(actual_type, None | Some(DataType::Unknown));
    assert!(
        is_unknown,
        "expected untyped function call to produce Unknown or no type, got {:?}",
        actual_type,
    );
}
