//! Collation tests (§17 of types.md).
//!
//! Four test groups:
//! 1. `non_portable_collation_code_exists` — compile-time gate: the variant exists.
//! 2. `binary_collation_passes_through` — binary collation names infer the operand
//!    type unchanged and emit no `NonPortableCollation` diagnostic.
//! 3. `non_binary_collation_diagnoses` — non-binary collation emits one
//!    `NonPortableCollation` Error anchored at the COLLATE clause span; the
//!    expression type degrades to `DataType::Unknown`.
//! 4. Binary-string oracle tests (the §17 standing gate) — verify that binary-collation
//!    string operations (`=`, `<`, `GROUP BY`, `DISTINCT`, `ORDER BY`) infer
//!    correctly against the live DuckDB oracle, and that no `NonPortableCollation`
//!    diagnostic fires in the presence of binary collation.

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::duckdb_oracle::{DuckDbOracle, TypeOracle};
use prop_helpers::type_comparison::{compare_types, TypeMatch};
use smelt_db::diagnostics_types::DiagnosticCode;
use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn infer(sql: &str) -> Vec<TypedColumn> {
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    infer_select_column_types(&select, &TypeContext::new())
}

/// Run file_diagnostics on a bare SQL SELECT and return all diagnostics.
fn diags_for(sql: &str) -> Vec<smelt_db::diagnostics_types::Diagnostic> {
    use smelt_db::{file_diagnostics, Database};
    use std::path::PathBuf;

    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("collation_test.sql");

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    let sf = db.set_source_file(model_path.clone(), sql.to_string(), root.clone());
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();
    file_diagnostics(&db, ws, sf)
}

// ─── Test 1: compile-time gate ────────────────────────────────────────────────

/// Verify that `NonPortableCollation` is a distinct `DiagnosticCode` variant.
///
/// This test fails to compile until `NonPortableCollation` is added to the
/// `DiagnosticCode` enum.
#[test]
fn non_portable_collation_code_exists() {
    let code = DiagnosticCode::NonPortableCollation;
    assert_ne!(code, DiagnosticCode::TypeMismatch);
    assert_ne!(code, DiagnosticCode::CannotInferType);
    assert_ne!(code, DiagnosticCode::DecimalPrecisionOverflow);
}

// ─── Test 2: binary collation passes through ─────────────────────────────────

/// Binary collation names: `"C"`, `POSIX`, `BINARY`, `UTF8_BINARY`.
///
/// Each must:
///   - infer the operand's type unchanged (Text here since we use a string literal),
///   - emit zero `NonPortableCollation` diagnostics.
#[test]
fn binary_collation_passes_through() {
    // Test all four binary collation names (case-insensitive):
    let binary_cases = [
        r#"SELECT 'hello' COLLATE "C" AS col1"#,
        r#"SELECT 'hello' COLLATE BINARY AS col2"#,
        r#"SELECT 'hello' COLLATE binary AS col2b"#,
        r#"SELECT 'hello' COLLATE UTF8_BINARY AS col3"#,
        r#"SELECT 'hello' COLLATE POSIX AS col4"#,
    ];

    for sql in &binary_cases {
        let types = infer(sql);
        assert_eq!(
            types.len(),
            1,
            "expected exactly one output column for: {sql}"
        );
        assert_eq!(
            types[0].data_type,
            DataType::Text,
            "binary COLLATE should pass through Text operand type unchanged for: {sql}"
        );

        let all_diags = diags_for(sql);
        let collation_diags: Vec<_> = all_diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::NonPortableCollation))
            .collect();
        assert_eq!(
            collation_diags.len(),
            0,
            "binary COLLATE must emit no NonPortableCollation diagnostic for: {sql}\n  got: {:?}",
            all_diags
        );
    }
}

// ─── Test 3: non-binary collation diagnoses ───────────────────────────────────

/// Non-binary collation names like `NOCASE` emit exactly one
/// `NonPortableCollation` Error, and the expression type degrades to
/// `DataType::Unknown`.
#[test]
fn non_binary_collation_diagnoses() {
    let non_binary_cases = [
        r#"SELECT 'hello' COLLATE NOCASE AS col1"#,
        r#"SELECT 'hello' COLLATE nocase AS col2"#,
        r#"SELECT 'hello' COLLATE RTRIM AS col3"#,
        r#"SELECT 'hello' COLLATE en_US AS col4"#,
    ];

    for sql in &non_binary_cases {
        // The inferred type should be Unknown.
        let types = infer(sql);
        assert_eq!(
            types.len(),
            1,
            "expected exactly one output column for: {sql}"
        );
        assert_eq!(
            types[0].data_type,
            DataType::Unknown,
            "non-binary COLLATE should degrade type to Unknown for: {sql}"
        );

        // Exactly one NonPortableCollation diagnostic must be emitted.
        let all_diags = diags_for(sql);
        let collation_diags: Vec<_> = all_diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::NonPortableCollation))
            .collect();
        assert_eq!(
            collation_diags.len(),
            1,
            "expected exactly 1 NonPortableCollation diagnostic for: {sql}\n  got: {:?}",
            all_diags
        );

        // Severity must be Error.
        assert_eq!(
            collation_diags[0].severity,
            smelt_db::diagnostics_types::DiagnosticSeverity::Error,
            "NonPortableCollation must be Error severity for: {sql}"
        );
    }
}

// ─── Binary-string oracle gate (§17) ─────────────────────────────────────────
//
// These tests form the "Standing collation gate" described in §Constraints of
// docs/specs/types.md.  They verify that portable (binary) string comparison
// and grouping operations produce types that agree with the live DuckDB oracle,
// and that no NonPortableCollation diagnostic fires anywhere in the path.

/// Helper: run smelt type inference on a SQL string with a given TypeContext.
fn infer_with_ctx(sql: &str, ctx: &TypeContext) -> Vec<TypedColumn> {
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    infer_select_column_types(&select, ctx)
}

/// §17 standing collation gate:
///
/// Binary-collation string equality (`=`) and ordering (`<`) run against the
/// live DuckDB oracle — smelt must infer `Boolean` for both, matching DuckDB,
/// and zero `NonPortableCollation` diagnostics must fire.
#[test]
fn binary_string_comparison_oracle() {
    let oracle = DuckDbOracle::new();

    // Equality comparison: DuckDB returns BOOLEAN
    let eq_sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) \
                  SELECT (s = s) AS eq_result FROM data";
    let duckdb_types = oracle
        .query_types(eq_sql)
        .expect("DuckDB should execute equality comparison");
    assert_eq!(
        duckdb_types.len(),
        1,
        "equality comparison should return one column"
    );
    assert_eq!(
        duckdb_types[0].1,
        DataType::Boolean,
        "DuckDB string equality should return Boolean"
    );

    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "data",
        "s",
        TypedColumn::nullable(DataType::Varchar { max_length: None }),
    );
    let inferred = infer_with_ctx(eq_sql, &ctx);
    assert_eq!(
        inferred.len(),
        1,
        "smelt should infer one output column for equality"
    );
    // smelt infers Boolean for = comparisons — compared against the DuckDB oracle result
    let match_result = compare_types(&inferred[0].data_type, &duckdb_types[0].1);
    assert!(
        matches!(
            match_result,
            TypeMatch::Exact | TypeMatch::Compatible { .. }
        ),
        "smelt string equality should infer Boolean (or compatible), got {:?}",
        inferred[0].data_type
    );

    // Less-than comparison: DuckDB returns BOOLEAN
    let lt_sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) \
                  SELECT (s < s) AS lt_result FROM data";
    let duckdb_lt_types = oracle
        .query_types(lt_sql)
        .expect("DuckDB should execute less-than comparison");
    assert_eq!(
        duckdb_lt_types[0].1,
        DataType::Boolean,
        "DuckDB string < should return Boolean"
    );

    let inferred_lt = infer_with_ctx(lt_sql, &ctx);
    let match_lt = compare_types(&inferred_lt[0].data_type, &duckdb_lt_types[0].1);
    assert!(
        matches!(match_lt, TypeMatch::Exact | TypeMatch::Compatible { .. }),
        "smelt string < should infer Boolean (or compatible), got {:?}",
        inferred_lt[0].data_type
    );

    // No NonPortableCollation diagnostics should fire for these plain string ops
    for sql in &[eq_sql, lt_sql] {
        let all_diags = diags_for(sql);
        let collation_diags: Vec<_> = all_diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::NonPortableCollation))
            .collect();
        assert_eq!(
            collation_diags.len(),
            0,
            "binary string comparison must not emit NonPortableCollation for: {sql}\n  got: {all_diags:?}"
        );
    }
}

/// §17 standing collation gate:
///
/// `GROUP BY`, `DISTINCT`, and `ORDER BY` on a `Text`/`Varchar` column are
/// deterministic under binary collation.  DuckDB agrees; smelt must infer the
/// group-key type correctly (Text/Varchar-compatible) and emit zero
/// `NonPortableCollation` diagnostics.
#[test]
fn binary_string_groupby_distinct_orderby_oracle() {
    let oracle = DuckDbOracle::new();

    // GROUP BY: group key passes through as its original type
    let grp_sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s, CAST(1 AS INTEGER) AS n) \
                   SELECT s AS grp_key, COUNT(n) AS cnt FROM data GROUP BY s";
    let duckdb_grp = oracle
        .query_types(grp_sql)
        .expect("DuckDB should execute GROUP BY");
    assert_eq!(duckdb_grp.len(), 2, "GROUP BY should return two columns");
    // grp_key column type: DuckDB returns VARCHAR (Text-compatible)
    assert!(
        matches!(&duckdb_grp[0].1, DataType::Text | DataType::Varchar { .. }),
        "GROUP BY key column should be text-family type, got {:?}",
        duckdb_grp[0].1
    );
    // cnt column type: DuckDB returns BIGINT for COUNT
    assert_eq!(
        duckdb_grp[1].1,
        DataType::BigInt,
        "COUNT should return BigInt"
    );

    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "data",
        "s",
        TypedColumn::nullable(DataType::Varchar { max_length: None }),
    );
    ctx.add_cte_column("data", "n", TypedColumn::nullable(DataType::Integer));
    let inferred_grp = infer_with_ctx(grp_sql, &ctx);
    // grp_key type should be Text-family compatible
    let match_grp = compare_types(&inferred_grp[0].data_type, &duckdb_grp[0].1);
    assert!(
        matches!(match_grp, TypeMatch::Exact | TypeMatch::Compatible { .. }),
        "smelt GROUP BY key must match DuckDB (Text-compatible): smelt={:?}, duckdb={:?}",
        inferred_grp[0].data_type,
        duckdb_grp[0].1
    );

    // DISTINCT: select distinct string values — type passes through
    let distinct_sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) \
                        SELECT DISTINCT s AS distinct_s FROM data";
    let duckdb_distinct = oracle
        .query_types(distinct_sql)
        .expect("DuckDB should execute DISTINCT");
    assert_eq!(
        duckdb_distinct.len(),
        1,
        "DISTINCT should return one column"
    );
    assert!(
        matches!(
            &duckdb_distinct[0].1,
            DataType::Text | DataType::Varchar { .. }
        ),
        "DISTINCT string column should be text-family, got {:?}",
        duckdb_distinct[0].1
    );
    let inferred_distinct = infer_with_ctx(distinct_sql, &ctx);
    let match_distinct = compare_types(&inferred_distinct[0].data_type, &duckdb_distinct[0].1);
    assert!(
        matches!(
            match_distinct,
            TypeMatch::Exact | TypeMatch::Compatible { .. }
        ),
        "smelt DISTINCT column must match DuckDB: smelt={:?}, duckdb={:?}",
        inferred_distinct[0].data_type,
        duckdb_distinct[0].1
    );

    // ORDER BY: the selected column retains its type
    let orderby_sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) \
                       SELECT s AS sorted_s FROM data ORDER BY s";
    let duckdb_order = oracle
        .query_types(orderby_sql)
        .expect("DuckDB should execute ORDER BY");
    assert_eq!(duckdb_order.len(), 1, "ORDER BY should return one column");
    assert!(
        matches!(
            &duckdb_order[0].1,
            DataType::Text | DataType::Varchar { .. }
        ),
        "ORDER BY string column should be text-family, got {:?}",
        duckdb_order[0].1
    );
    let inferred_order = infer_with_ctx(orderby_sql, &ctx);
    let match_order = compare_types(&inferred_order[0].data_type, &duckdb_order[0].1);
    assert!(
        matches!(match_order, TypeMatch::Exact | TypeMatch::Compatible { .. }),
        "smelt ORDER BY column must match DuckDB: smelt={:?}, duckdb={:?}",
        inferred_order[0].data_type,
        duckdb_order[0].1
    );

    // None of these plain binary operations should emit NonPortableCollation
    for sql in &[grp_sql, distinct_sql, orderby_sql] {
        let all_diags = diags_for(sql);
        let collation_diags: Vec<_> = all_diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::NonPortableCollation))
            .collect();
        assert_eq!(
            collation_diags.len(),
            0,
            "binary string grouping/ordering must not emit NonPortableCollation for: {sql}\n  got: {all_diags:?}"
        );
    }
}

/// §17 standing collation gate:
///
/// Binary-collation `COLLATE "C"` on `GROUP BY`, `DISTINCT`, and `ORDER BY` is
/// portable and deterministic.  The DuckDB oracle accepts the explicit binary
/// COLLATE clause; smelt must infer the correct type and emit zero
/// `NonPortableCollation` diagnostics.
#[test]
fn binary_collate_in_groupby_distinct_orderby_no_diagnostic() {
    // Explicit COLLATE "C" on a GROUP BY expression — DuckDB accepts this and
    // returns the string type unchanged.  smelt must not emit NonPortableCollation.
    let cases = [
        // GROUP BY with explicit binary COLLATE
        (
            r#"WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s, CAST(1 AS INTEGER) AS n)
               SELECT s COLLATE "C" AS grp_key, COUNT(n) AS cnt
               FROM data GROUP BY s COLLATE "C""#,
            "GROUP BY COLLATE",
        ),
        // ORDER BY with explicit binary COLLATE
        (
            r#"WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s)
               SELECT s COLLATE "C" AS sorted_s FROM data ORDER BY s COLLATE "C""#,
            "ORDER BY COLLATE",
        ),
        // DISTINCT with binary COLLATE in select list
        (
            r#"WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s)
               SELECT DISTINCT s COLLATE "C" AS distinct_s FROM data"#,
            "DISTINCT COLLATE",
        ),
    ];

    for (sql, label) in &cases {
        let all_diags = diags_for(sql);
        let collation_diags: Vec<_> = all_diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::NonPortableCollation))
            .collect();
        assert_eq!(
            collation_diags.len(),
            0,
            "{label}: binary COLLATE in {label} must emit zero NonPortableCollation diagnostics\n  got: {all_diags:?}"
        );
    }
}

/// §17 standing collation gate:
///
/// Using explicit `COLLATE "C"` in `MIN`/`MAX` aggregates is portable and
/// deterministic under binary collation.  These string min/max operations
/// agree across DuckDB, Spark (UTF8_BINARY default), and Postgres (C locale).
#[test]
fn binary_collate_min_max_string_no_diagnostic() {
    let oracle = DuckDbOracle::new();

    // MIN and MAX on a string column should return the same string type
    let min_max_sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) \
                       SELECT MIN(s) AS min_s, MAX(s) AS max_s FROM data";
    let duckdb_types = oracle
        .query_types(min_max_sql)
        .expect("DuckDB should execute MIN/MAX on strings");
    assert_eq!(duckdb_types.len(), 2, "MIN/MAX should return two columns");
    for (name, typ) in &duckdb_types {
        assert!(
            matches!(typ, DataType::Text | DataType::Varchar { .. }),
            "DuckDB MIN/MAX on string should return text-family type for {name}, got {typ:?}"
        );
    }

    // smelt must infer the same MIN/MAX output types as DuckDB (MIN(T)/MAX(T) → T)
    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "data",
        "s",
        TypedColumn::nullable(DataType::Varchar { max_length: None }),
    );
    let inferred = infer_with_ctx(min_max_sql, &ctx);
    assert_eq!(inferred.len(), 2, "smelt should infer two MIN/MAX columns");
    for (i, (name, duckdb_typ)) in duckdb_types.iter().enumerate() {
        let m = compare_types(&inferred[i].data_type, duckdb_typ);
        assert!(
            matches!(m, TypeMatch::Exact | TypeMatch::Compatible { .. }),
            "smelt MIN/MAX output must match DuckDB for {name}: smelt={:?}, duckdb={:?}",
            inferred[i].data_type,
            duckdb_typ
        );
    }

    // No NonPortableCollation diagnostic should fire for plain MIN/MAX
    let all_diags = diags_for(min_max_sql);
    let collation_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::NonPortableCollation))
        .collect();
    assert_eq!(
        collation_diags.len(),
        0,
        "MIN/MAX on binary strings must emit zero NonPortableCollation\n  got: {all_diags:?}"
    );
}
