//! Integration tests for CTE column-list rebinding.
//!
//! `WITH cte(a, b) AS (SELECT …)` rebinds the inner SELECT's column types
//! to the declared names positionally.  When no column list is given, the
//! inner SELECT's own alias names are used unchanged (existing behavior).
//!
//! Tests drive the real `typed_model_schema` Salsa query.
//!
//! Cases:
//!   1. `cte_with_explicit_column_list` — `WITH cte(a, b) AS (SELECT 1, 2.0)` →
//!      `{a: Integer, b: Double}`.
//!   2. `cte_without_column_list_regression` — `WITH cte AS (SELECT 1 AS x)`
//!      → `{x: Integer}` (existing behavior; regression guard).
//!   3. `cte_mixed_types_preserved_on_rebinding` — `WITH cte(price) AS
//!      (SELECT CAST(1.5 AS DOUBLE))` → `{price: Double}`.

use std::path::PathBuf;

use smelt_db::{typed_model_schema, Database, Workspace};
use smelt_types::DataType;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn build_db_single_model(model_sql: &str) -> (Database, Workspace, smelt_db::SourceFile) {
    let project_root = PathBuf::from("/fake/cte_rebind_test");
    let model_path = project_root.join("models").join("model.sql");

    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let sf = db.set_source_file(
        model_path.clone(),
        model_sql.to_string(),
        project_root.clone(),
    );
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();
    (db, ws, sf)
}

/// Extract a map of column name -> DataType from typed_model_schema.
fn column_types(
    db: &Database,
    ws: Workspace,
    sf: smelt_db::SourceFile,
) -> std::collections::HashMap<String, DataType> {
    let schema = typed_model_schema(db, ws, sf);
    schema
        .columns
        .iter()
        .filter_map(|c| {
            c.data_type
                .as_ref()
                .map(|dt| (c.name.clone(), dt.data_type.clone()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Test 1: explicit CTE column list rebinds positionally
// ---------------------------------------------------------------------------

/// `WITH cte(a, b) AS (SELECT CAST(1 AS INTEGER), CAST(2.0 AS DOUBLE)) SELECT a, b FROM cte`
/// The SELECT names the columns positionally to `a` and `b`.
/// Output schema of the outer model must show `a: Integer, b: Double`.
#[test]
fn cte_with_explicit_column_list() {
    let sql =
        "WITH cte(a, b) AS (SELECT CAST(1 AS INTEGER), CAST(2.0 AS DOUBLE)) SELECT a, b FROM cte\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let types = column_types(&db, ws, sf);

    assert_eq!(
        types.get("a"),
        Some(&DataType::Integer),
        "expected a=Integer, got {:?}; full map: {:?}",
        types.get("a"),
        types
    );

    assert_eq!(
        types.get("b"),
        Some(&DataType::Double),
        "expected b=Double, got {:?}; full map: {:?}",
        types.get("b"),
        types
    );
}

// ---------------------------------------------------------------------------
// Test 2: no CTE column list → inner alias names unchanged (regression)
// ---------------------------------------------------------------------------

/// `WITH cte AS (SELECT CAST(1 AS INTEGER) AS x) SELECT x FROM cte`
/// Without a column list, `x` keeps the name `x` from the inner SELECT.
#[test]
fn cte_without_column_list_regression() {
    let sql = "WITH cte AS (SELECT CAST(1 AS INTEGER) AS x) SELECT x FROM cte\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let types = column_types(&db, ws, sf);

    assert_eq!(
        types.get("x"),
        Some(&DataType::Integer),
        "expected x=Integer, got {:?}; full map: {:?}",
        types.get("x"),
        types
    );
}

// ---------------------------------------------------------------------------
// Test 3: explicit list preserves type, only renames
// ---------------------------------------------------------------------------

/// `WITH cte(price) AS (SELECT CAST(1.5 AS DOUBLE) AS raw_price) SELECT price FROM cte`
/// The declared name `price` replaces the inner alias `raw_price`; the type (Double)
/// must be preserved.
#[test]
fn cte_mixed_types_preserved_on_rebinding() {
    let sql =
        "WITH cte(price) AS (SELECT CAST(1.5 AS DOUBLE) AS raw_price) SELECT price FROM cte\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let types = column_types(&db, ws, sf);

    assert_eq!(
        types.get("price"),
        Some(&DataType::Double),
        "expected price=Double, got {:?}; full map: {:?}",
        types.get("price"),
        types
    );
}
