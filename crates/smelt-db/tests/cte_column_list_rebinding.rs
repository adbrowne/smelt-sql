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

use smelt_db::{file_diagnostics, typed_model_schema, Database, DiagnosticCode, Workspace};
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

// ---------------------------------------------------------------------------
// Helper: collect all diagnostics (file_diagnostics + check_type_diagnostics).
// ---------------------------------------------------------------------------

fn all_diagnostics(
    db: &Database,
    ws: Workspace,
    sf: smelt_db::SourceFile,
) -> Vec<smelt_db::Diagnostic> {
    use smelt_db::DiagnosticAcc;
    let mut diags = file_diagnostics(db, ws, sf);
    for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(db, ws, sf) {
        diags.push(d.0.clone());
    }
    diags
}

// ---------------------------------------------------------------------------
// Phase 3 TDD: AliasColumnArityMismatch for CTE column lists
// ---------------------------------------------------------------------------

/// `WITH cte(a) AS (SELECT 1, 2) SELECT * FROM cte`
/// One declared column name but two inner SELECT columns → mismatch.
/// Must emit exactly one `AliasColumnArityMismatch` diagnostic.
#[test]
fn cte_alias_too_few_names_emits_arity_mismatch() {
    let sql = "WITH cte(a) AS (SELECT CAST(1 AS INTEGER), CAST(2 AS INTEGER)) SELECT a FROM cte\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let diags = all_diagnostics(&db, ws, sf);

    let arity_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::AliasColumnArityMismatch))
        .collect();

    assert_eq!(
        arity_diags.len(),
        1,
        "expected exactly 1 AliasColumnArityMismatch, got {}:\n  {:?}",
        arity_diags.len(),
        diags
    );
}

/// `WITH cte(a, b) AS (SELECT 1) SELECT * FROM cte`
/// Two declared column names but one inner SELECT column → mismatch.
/// Must emit exactly one `AliasColumnArityMismatch` diagnostic.
#[test]
fn cte_alias_too_many_names_emits_arity_mismatch() {
    let sql = "WITH cte(a, b) AS (SELECT CAST(1 AS INTEGER)) SELECT a FROM cte\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let diags = all_diagnostics(&db, ws, sf);

    let arity_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::AliasColumnArityMismatch))
        .collect();

    assert_eq!(
        arity_diags.len(),
        1,
        "expected exactly 1 AliasColumnArityMismatch, got {}:\n  {:?}",
        arity_diags.len(),
        diags
    );
}

/// Regression: `WITH cte(a, b) AS (SELECT 1, 2) SELECT a, b FROM cte`
/// Matching alias count must emit zero `AliasColumnArityMismatch` diagnostics.
#[test]
fn cte_alias_matching_count_no_arity_mismatch() {
    let sql =
        "WITH cte(a, b) AS (SELECT CAST(1 AS INTEGER), CAST(2 AS INTEGER)) SELECT a, b FROM cte\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let diags = all_diagnostics(&db, ws, sf);

    let arity_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::AliasColumnArityMismatch))
        .collect();

    assert_eq!(
        arity_diags.len(),
        0,
        "expected zero AliasColumnArityMismatch for matching alias count, got {}:\n  {:?}",
        arity_diags.len(),
        diags
    );
}

/// Regression: `WITH cte AS (SELECT 1 AS x) SELECT x FROM cte`
/// No alias list → no `AliasColumnArityMismatch` diagnostic.
#[test]
fn cte_no_alias_list_no_arity_mismatch() {
    let sql = "WITH cte AS (SELECT CAST(1 AS INTEGER) AS x) SELECT x FROM cte\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let diags = all_diagnostics(&db, ws, sf);

    let arity_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::AliasColumnArityMismatch))
        .collect();

    assert_eq!(
        arity_diags.len(),
        0,
        "expected zero AliasColumnArityMismatch when no alias list, got {}:\n  {:?}",
        arity_diags.len(),
        diags
    );
}
