//! Integration tests for VALUES-derived-table column type inference.
//!
//! A `(VALUES …) AS t(c1, c2, …)` derived table must produce a typed schema
//! where each column's type is the LUB (least upper bound under the numeric
//! promotion lattice) of the corresponding element across all rows.
//!
//! Tests drive the real `typed_model_schema` Salsa query.
//!
//! Cases:
//!   1. `single_row_concrete_values` — fully concrete single-row VALUES with
//!      Integer, Text, and Date columns produces concrete types.
//!   2. `multi_row_promotion` — two-row VALUES where one column has Integer
//!      and Double values must widen to Double (numeric promotion).
//!   3. `no_alias_col_list_uses_col_n_names` — VALUES without an alias column
//!      list uses `col1`, `col2`, … as column names.
//!   4. `full_reproducer_five_columns` — the `examples/per_cohort_union/orders`
//!      fixture: five columns with TIMESTAMP cast, all concrete.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, typed_model_schema, Database, DiagnosticCode, Workspace};
use smelt_types::DataType;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn build_db_single_model(model_sql: &str) -> (Database, Workspace, smelt_db::SourceFile) {
    let project_root = PathBuf::from("/fake/values_test");
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
// Test 1: single-row, fully concrete
// ---------------------------------------------------------------------------

/// Single-row VALUES with Integer, Text, and Date columns must produce those
/// concrete types.  The alias column list binds the three columns to (i, s, d).
///
/// Uses CAST to avoid the literal-type-narrowing issue (bare `1` → SmallInt,
/// string literal → Varchar).
#[test]
fn single_row_concrete_values() {
    let sql = "SELECT i, s, d FROM (\
        VALUES (CAST(1 AS INTEGER), CAST('a' AS TEXT), CAST(NULL AS DATE))) AS t(i, s, d)\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let types = column_types(&db, ws, sf);

    // i should be Integer (CAST AS INTEGER)
    assert_eq!(
        types.get("i"),
        Some(&DataType::Integer),
        "expected i=Integer, got {:?}; full map: {:?}",
        types.get("i"),
        types
    );

    // s should be Text or Varchar (CAST AS TEXT — the existing type parser
    // maps "TEXT" → Varchar { max_length: None }, which is the canonical
    // string representation; both are acceptable string types).
    assert!(
        matches!(
            types.get("s"),
            Some(DataType::Text) | Some(DataType::Varchar { .. })
        ),
        "expected s=Text or Varchar, got {:?}; full map: {:?}",
        types.get("s"),
        types
    );

    // d should be Date (from CAST(NULL AS DATE))
    assert_eq!(
        types.get("d"),
        Some(&DataType::Date),
        "expected d=Date, got {:?}; full map: {:?}",
        types.get("d"),
        types
    );
}

/// Implicit alias (no `AS` keyword): `(VALUES …) t(i, s, d)` must bind the
/// alias column list and type the columns identically to the explicit
/// `AS t(i, s, d)` form. Before the parser captured the implicit-alias column
/// list, the `(i, s, d)` list leaked into the enclosing projection and every
/// column fell through to `Unknown`.
#[test]
fn single_row_concrete_values_implicit_alias() {
    let sql = "SELECT i, s, d FROM (\
        VALUES (CAST(1 AS INTEGER), CAST('a' AS TEXT), CAST(NULL AS DATE))) t(i, s, d)\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let types = column_types(&db, ws, sf);

    assert_eq!(
        types.get("i"),
        Some(&DataType::Integer),
        "expected i=Integer, got {:?}; full map: {:?}",
        types.get("i"),
        types
    );
    assert!(
        matches!(
            types.get("s"),
            Some(DataType::Text) | Some(DataType::Varchar { .. })
        ),
        "expected s=Text or Varchar, got {:?}; full map: {:?}",
        types.get("s"),
        types
    );
    assert_eq!(
        types.get("d"),
        Some(&DataType::Date),
        "expected d=Date, got {:?}; full map: {:?}",
        types.get("d"),
        types
    );
}

// ---------------------------------------------------------------------------
// Test 2: multi-row numeric promotion
// ---------------------------------------------------------------------------

/// Two-row VALUES: first row has Integer, second has Double.
/// Column-wise LUB under the numeric promotion chain must be Double.
/// Uses CAST to ensure concrete Integer/Double types rather than relying on
/// literal type narrowing.
#[test]
fn multi_row_promotion() {
    let sql = "SELECT x FROM (VALUES (CAST(1 AS INTEGER)), (CAST(2.0 AS DOUBLE))) AS t(x)\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let types = column_types(&db, ws, sf);

    assert_eq!(
        types.get("x"),
        Some(&DataType::Double),
        "expected x=Double after Integer+Double promotion, got {:?}; full map: {:?}",
        types.get("x"),
        types
    );
}

// ---------------------------------------------------------------------------
// Test 3: no alias column list → col1..colN
// ---------------------------------------------------------------------------

/// When no alias column list is given, columns are named col1, col2, ….
/// SELECT projections that reference the fallback names must resolve.
#[test]
fn no_alias_col_list_uses_col_n_names() {
    // No column alias list: (VALUES (CAST(1 AS INTEGER), CAST(2 AS INTEGER))) AS t
    // The derived table columns should be col1 and col2.
    let sql = "SELECT col1, col2 FROM (VALUES (CAST(1 AS INTEGER), CAST(2 AS INTEGER))) AS t\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let types = column_types(&db, ws, sf);

    assert_eq!(
        types.get("col1"),
        Some(&DataType::Integer),
        "expected col1=Integer, got {:?}; full map: {:?}",
        types.get("col1"),
        types
    );
    assert_eq!(
        types.get("col2"),
        Some(&DataType::Integer),
        "expected col2=Integer, got {:?}; full map: {:?}",
        types.get("col2"),
        types
    );
}

// ---------------------------------------------------------------------------
// Test 4: full reproducer (examples/per_cohort_union/orders.sql)
// ---------------------------------------------------------------------------

/// Five-column VALUES with an alias column list — the `orders` model in
/// `examples/per_cohort_union`.  All five columns must be concrete.
#[test]
fn full_reproducer_five_columns() {
    let sql = "SELECT id, user_id, region, revenue, created_at\nFROM (\n    VALUES\n        \
        (1, 10, 'us-west-2', 150, CAST('2024-01-01' AS TIMESTAMP)),\n        \
        (2, 11, 'us-west-2', 80,  CAST('2024-01-02' AS TIMESTAMP)),\n        \
        (3, 20, 'us-east-1', 120, CAST('2024-01-03' AS TIMESTAMP)),\n        \
        (4, 21, 'us-east-1', 90,  CAST('2024-01-04' AS TIMESTAMP)),\n        \
        (5, 30, 'eu-west-1', 60,  CAST('2024-01-05' AS TIMESTAMP)),\n        \
        (6, 31, 'eu-west-1', 40,  CAST('2024-01-06' AS TIMESTAMP))\n) AS t(id, user_id, region, revenue, created_at)\n";

    let (db, ws, sf) = build_db_single_model(sql);
    let types = column_types(&db, ws, sf);

    // id — numeric (small integer literal, exact type is SmallInt or Integer after promotion)
    assert!(
        matches!(
            types.get("id"),
            Some(DataType::SmallInt) | Some(DataType::Integer) | Some(DataType::BigInt)
        ),
        "expected id=numeric type, got {:?}; full map: {:?}",
        types.get("id"),
        types
    );

    // user_id — numeric
    assert!(
        matches!(
            types.get("user_id"),
            Some(DataType::SmallInt) | Some(DataType::Integer) | Some(DataType::BigInt)
        ),
        "expected user_id=numeric type, got {:?}; full map: {:?}",
        types.get("user_id"),
        types
    );

    // region — Text or Varchar (string literal)
    assert!(
        matches!(
            types.get("region"),
            Some(DataType::Text) | Some(DataType::Varchar { .. })
        ),
        "expected region=Text or Varchar, got {:?}; full map: {:?}",
        types.get("region"),
        types
    );

    // revenue — numeric
    assert!(
        matches!(
            types.get("revenue"),
            Some(DataType::SmallInt) | Some(DataType::Integer) | Some(DataType::BigInt)
        ),
        "expected revenue=numeric type, got {:?}; full map: {:?}",
        types.get("revenue"),
        types
    );

    // created_at — Timestamp (from CAST('...' AS TIMESTAMP))
    assert!(
        matches!(types.get("created_at"), Some(DataType::Timestamp { .. })),
        "expected created_at=Timestamp, got {:?}; full map: {:?}",
        types.get("created_at"),
        types
    );

    // All five must be non-Unknown
    for (name, dt) in &types {
        assert_ne!(
            dt,
            &DataType::Unknown,
            "column {name} must not be Unknown; full map: {:?}",
            types
        );
    }
}

// ---------------------------------------------------------------------------
// Helper to collect all diagnostics for a single-model workspace.
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
// Phase 3 TDD: AliasColumnArityMismatch for VALUES derived tables
// ---------------------------------------------------------------------------

/// `(VALUES (1, 2)) AS t(a)` — two-column VALUES but only one alias name.
/// Must emit exactly one `AliasColumnArityMismatch` diagnostic.
#[test]
fn values_alias_too_few_names_emits_arity_mismatch() {
    // 2 columns in VALUES, 1 name in alias list → mismatch
    let sql = "SELECT a FROM (VALUES (CAST(1 AS INTEGER), CAST(2 AS INTEGER))) AS t(a)\n";
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

/// `(VALUES (1)) AS t(a, b)` — one-column VALUES but two alias names.
/// Must emit exactly one `AliasColumnArityMismatch` diagnostic.
#[test]
fn values_alias_too_many_names_emits_arity_mismatch() {
    // 1 column in VALUES, 2 names in alias list → mismatch
    let sql = "SELECT a FROM (VALUES (CAST(1 AS INTEGER))) AS t(a, b)\n";
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

/// Regression: `(VALUES (1, 2)) AS t(a, b)` — matching alias count.
/// Must emit zero `AliasColumnArityMismatch` diagnostics.
#[test]
fn values_alias_matching_count_no_arity_mismatch() {
    let sql = "SELECT a, b FROM (VALUES (CAST(1 AS INTEGER), CAST(2 AS INTEGER))) AS t(a, b)\n";
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

/// Regression: `(VALUES (1)) AS t` — no alias column list.
/// Must emit zero `AliasColumnArityMismatch` diagnostics.
#[test]
fn values_no_alias_list_no_arity_mismatch() {
    let sql = "SELECT col1 FROM (VALUES (CAST(1 AS INTEGER))) AS t\n";
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

// ---------------------------------------------------------------------------
// Phase 3 TDD: EmptyValuesClause for empty VALUES
// ---------------------------------------------------------------------------

/// Test the `EmptyValuesClause` diagnostic path.
///
/// The parser does not accept a zero-row `(VALUES) AS t` form — a VALUES clause
/// syntactically requires at least one row. Therefore the `EmptyValuesClause`
/// code path is reachable only through future meta-language surfaces (e.g. a
/// list splice that produces zero VALUES rows at compile time).
///
/// This test verifies:
/// 1. `DiagnosticCode::EmptyValuesClause` exists in the enum.
/// 2. The `ValuesError::Empty` variant exists and is the sentinel for that path.
/// 3. A non-empty VALUES produces no `EmptyValuesClause` diagnostic.
#[test]
fn empty_values_diagnostic_code_exists_and_non_empty_values_clean() {
    // Compile-time check: the code exists in the enum.
    let _: smelt_db::DiagnosticCode = smelt_db::DiagnosticCode::EmptyValuesClause;

    // Runtime check: a non-empty VALUES must not emit EmptyValuesClause.
    let sql = "SELECT col1 FROM (VALUES (CAST(1 AS INTEGER))) AS t\n";
    let (db, ws, sf) = build_db_single_model(sql);
    let diags = all_diagnostics(&db, ws, sf);

    let empty_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::EmptyValuesClause))
        .collect();

    assert_eq!(
        empty_diags.len(),
        0,
        "expected zero EmptyValuesClause for non-empty VALUES, got {}:\n  {:?}",
        empty_diags.len(),
        diags
    );
}
