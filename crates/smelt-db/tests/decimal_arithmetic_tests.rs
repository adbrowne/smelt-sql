//! Phase 1+2+3+5: Decimal arithmetic type inference tests.
//!
//! Phase 1 (already passing): `DecimalPrecisionOverflow` diagnostic code exists.
//! Phase 2 (TDD): Growth formulas for Decimal arithmetic, integer lifting,
//! and overflow detection.
//! Phase 3 (TDD): Decimal division rejection — emits TypeMismatch and returns Unknown.
//! Phase 5 (TDD): ABS(Decimal(p,s)) → Decimal(p,s) preserves precision and scale.

use smelt_db::diagnostics_types::DiagnosticCode;
use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

/// Verify that `DecimalPrecisionOverflow` is a distinct `DiagnosticCode` variant.
///
/// This test fails to compile until `DecimalPrecisionOverflow` is added to the
/// enum.  It is the red half of the TDD cycle for the overflow diagnostic code.
#[test]
fn decimal_precision_overflow_code_exists() {
    let code = DiagnosticCode::DecimalPrecisionOverflow;
    assert_ne!(code, DiagnosticCode::TypeMismatch);
    assert_ne!(code, DiagnosticCode::CannotInferType);
}

/// Parse a SELECT statement and infer column types using an empty context.
fn infer(sql: &str) -> Vec<TypedColumn> {
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    infer_select_column_types(&select, &TypeContext::new())
}

/// Phase 2 TDD test 1: Decimal + Decimal uses the growth formula.
///
/// `DECIMAL(10,2) + DECIMAL(5,1)`:
///   p' = max(p1-s1, p2-s2) + max(s1, s2) + 1
///      = max(10-2, 5-1) + max(2, 1) + 1
///      = max(8, 4) + 2 + 1
///      = 8 + 2 + 1 = 11
///   s' = max(s1, s2) = max(2, 1) = 2
/// Expected: Decimal(11, 2)
#[test]
fn decimal_add_growth_formula() {
    let sql = "SELECT CAST(1 AS DECIMAL(10,2)) + CAST(1 AS DECIMAL(5,1)) AS result";
    let types = infer(sql);
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Decimal {
            precision: 11,
            scale: 2
        },
        "DECIMAL(10,2) + DECIMAL(5,1) should yield DECIMAL(11,2) via growth formula"
    );
}

/// Phase 2 TDD test 2: Decimal * Decimal uses the multiplication growth formula.
///
/// `DECIMAL(10,2) * DECIMAL(5,1)`:
///   p' = p1 + p2 + 1 = 10 + 5 + 1 = 16
///   s' = s1 + s2 = 2 + 1 = 3
/// Expected: Decimal(16, 3)
#[test]
fn decimal_mul_growth_formula() {
    let sql = "SELECT CAST(1 AS DECIMAL(10,2)) * CAST(1 AS DECIMAL(5,1)) AS result";
    let types = infer(sql);
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Decimal {
            precision: 16,
            scale: 3
        },
        "DECIMAL(10,2) * DECIMAL(5,1) should yield DECIMAL(16,3) via multiplication growth formula"
    );
}

/// Phase 2 TDD test 3: Integer + Decimal uses integer lifting then growth formula.
///
/// INTEGER lifts to DECIMAL(10, 0).
/// `DECIMAL(10,0) + DECIMAL(10,2)`:
///   p' = max(10-0, 10-2) + max(0, 2) + 1
///      = max(10, 8) + 2 + 1
///      = 10 + 2 + 1 = 13
///   s' = max(0, 2) = 2
/// Expected: Decimal(13, 2)
#[test]
fn integer_lifting_add_decimal() {
    let sql = "SELECT CAST(1 AS INTEGER) + CAST(1 AS DECIMAL(10,2)) AS result";
    let types = infer(sql);
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Decimal {
            precision: 13,
            scale: 2
        },
        "INTEGER + DECIMAL(10,2) should lift INTEGER to DECIMAL(10,0) then yield DECIMAL(13,2)"
    );
}

/// Phase 2 TDD test 4: Decimal overflow emits a diagnostic and returns Unknown.
///
/// `DECIMAL(30,2) * DECIMAL(30,2)`:
///   p' = 30 + 30 + 1 = 61 > 38
/// Expected: result type is Unknown, exactly one `DecimalPrecisionOverflow` diagnostic.
#[test]
fn decimal_overflow_check_emits_diagnostic() {
    use smelt_db::{file_diagnostics, Database};
    use std::path::PathBuf;

    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("overflow_test.sql");
    let model_src =
        "--- name: overflow_test\nSELECT CAST(1 AS DECIMAL(30,2)) * CAST(1 AS DECIMAL(30,2)) AS result\n---\n";

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    let sf = db.set_source_file(model_path.clone(), model_src.to_string(), root.clone());
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();

    let all_diags = file_diagnostics(&db, ws, sf);
    let overflow_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::DecimalPrecisionOverflow))
        .collect();

    assert_eq!(
        overflow_diags.len(),
        1,
        "expected exactly one DecimalPrecisionOverflow diagnostic; got: {:?}",
        all_diags
    );

    // The result type should be Unknown (inferred from column schema)
    let parse =
        smelt_parser::parse("SELECT CAST(1 AS DECIMAL(30,2)) * CAST(1 AS DECIMAL(30,2)) AS result");
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    let types = infer_select_column_types(&select, &TypeContext::new());
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Unknown,
        "overflow result should be Unknown"
    );
}

// ─── Phase 3: Decimal division rejection ─────────────────────────────────────

/// Helper: run file_diagnostics on a bare SQL SELECT (no name header needed).
fn diags_for_model(model_src: &str) -> Vec<smelt_db::diagnostics_types::Diagnostic> {
    use smelt_db::{file_diagnostics, Database};
    use std::path::PathBuf;

    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("test_model.sql");

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    let sf = db.set_source_file(model_path.clone(), model_src.to_string(), root.clone());
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();
    file_diagnostics(&db, ws, sf)
}

/// Phase 3 TDD test 1: Decimal / Decimal emits TypeMismatch and returns Unknown.
///
/// The TypeMismatch message must contain "Double" (the cast remedy).
/// The inferred result type must be Unknown.
#[test]
fn decimal_division_emits_type_mismatch() {
    let model_src = "--- name: test_model\nSELECT CAST(1 AS DECIMAL(10,2)) / CAST(1 AS DECIMAL(5,1)) AS result\n---\n";
    let all_diags = diags_for_model(model_src);
    let type_mismatch_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::TypeMismatch))
        .collect();

    assert_eq!(
        type_mismatch_diags.len(),
        1,
        "expected exactly one TypeMismatch diagnostic for Decimal / Decimal; got: {:?}",
        all_diags
    );
    assert!(
        type_mismatch_diags[0].message.contains("Double"),
        "TypeMismatch message should mention 'Double' as the cast remedy; got: {}",
        type_mismatch_diags[0].message
    );

    // The result type should be Unknown
    let sql = "SELECT CAST(1 AS DECIMAL(10,2)) / CAST(1 AS DECIMAL(5,1)) AS result";
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    let types = infer_select_column_types(&select, &TypeContext::new());
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Unknown,
        "Decimal / Decimal result should be Unknown"
    );
}

/// Phase 3 TDD test 2: Decimal / Integer emits TypeMismatch and returns Unknown.
#[test]
fn decimal_integer_division_rejected() {
    let model_src = "--- name: test_model\nSELECT CAST(1 AS DECIMAL(10,2)) / CAST(1 AS INTEGER) AS result\n---\n";
    let all_diags = diags_for_model(model_src);
    let type_mismatch_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::TypeMismatch))
        .collect();

    assert_eq!(
        type_mismatch_diags.len(),
        1,
        "expected exactly one TypeMismatch diagnostic for Decimal / Integer; got: {:?}",
        all_diags
    );
    assert!(
        type_mismatch_diags[0].message.contains("Double"),
        "TypeMismatch message should mention 'Double' as the cast remedy; got: {}",
        type_mismatch_diags[0].message
    );

    // The result type should be Unknown
    let sql = "SELECT CAST(1 AS DECIMAL(10,2)) / CAST(1 AS INTEGER) AS result";
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    let types = infer_select_column_types(&select, &TypeContext::new());
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Unknown,
        "Decimal / Integer result should be Unknown"
    );
}

/// Phase 3 TDD test 3: Integer / Integer is still truncating division — no TypeMismatch.
#[test]
fn integer_division_still_truncating() {
    let model_src =
        "--- name: test_model\nSELECT CAST(7 AS INTEGER) / CAST(2 AS INTEGER) AS result\n---\n";
    let all_diags = diags_for_model(model_src);
    let type_mismatch_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::TypeMismatch))
        .collect();

    assert!(
        type_mismatch_diags.is_empty(),
        "Integer / Integer should not emit TypeMismatch; got: {:?}",
        all_diags
    );

    // The result type should be Integer (truncating division — spec §3)
    let sql = "SELECT CAST(7 AS INTEGER) / CAST(2 AS INTEGER) AS result";
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    let types = infer_select_column_types(&select, &TypeContext::new());
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Integer,
        "Integer / Integer should yield Integer (truncating division)"
    );
}

/// Phase 3 TDD test 4: Double / Double is still fine — no TypeMismatch.
#[test]
fn double_division_still_works() {
    let model_src =
        "--- name: test_model\nSELECT CAST(7.0 AS DOUBLE) / CAST(2.0 AS DOUBLE) AS result\n---\n";
    let all_diags = diags_for_model(model_src);
    let type_mismatch_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::TypeMismatch))
        .collect();

    assert!(
        type_mismatch_diags.is_empty(),
        "Double / Double should not emit TypeMismatch; got: {:?}",
        all_diags
    );

    // The result type should be Double
    let sql = "SELECT CAST(7.0 AS DOUBLE) / CAST(2.0 AS DOUBLE) AS result";
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    let types = infer_select_column_types(&select, &TypeContext::new());
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Double,
        "Double / Double should yield Double"
    );
}

// ─── Phase 5: ABS(Decimal) preserves precision and scale ────────────────────

/// Phase 5 TDD test 1: ABS(Decimal(p,s)) preserves precision and scale.
///
/// Per spec §15: `ABS(Decimal(p, s)) → Decimal(p, s)` — absolute value
/// preserves precision and scale. Previously returned `Unknown` because
/// the Numeric-generic signature did not thread precision/scale.
#[test]
fn abs_decimal_preserves_precision_scale() {
    let sql = "SELECT ABS(CAST(-1.23 AS DECIMAL(10,2))) AS result";
    let types = infer(sql);
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Decimal {
            precision: 10,
            scale: 2
        },
        "ABS(DECIMAL(10,2)) should yield DECIMAL(10,2), preserving precision and scale; got: {:?}",
        types[0].data_type
    );
}

/// Phase 5 TDD test 2: ABS on non-Decimal numeric type is unaffected.
///
/// Regression guard: ABS(Integer) must still return Integer (not Decimal).
#[test]
fn abs_integer_unaffected() {
    let sql = "SELECT ABS(CAST(-1 AS INTEGER)) AS result";
    let types = infer(sql);
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Integer,
        "ABS(INTEGER) should yield INTEGER, not Decimal; got: {:?}",
        types[0].data_type
    );
}

/// Phase 5 TDD test 3: ABS on a wider Decimal type (DECIMAL(18,2)).
///
/// Verifies the general case works beyond the simple (10,2) example.
#[test]
fn abs_decimal_wide_preserves_precision_scale() {
    let sql = "SELECT ABS(CAST(-1 AS DECIMAL(18,2))) AS result";
    let types = infer(sql);
    assert_eq!(types.len(), 1, "expected exactly one output column");
    assert_eq!(
        types[0].data_type,
        DataType::Decimal {
            precision: 18,
            scale: 2
        },
        "ABS(DECIMAL(18,2)) should yield DECIMAL(18,2); got: {:?}",
        types[0].data_type
    );
}

/// Phase 5 TDD test 4: ABS on schema-resolved Decimal column via cross-model reference.
///
/// This reproduces the multi-model proptest failure. When dec_col_3 comes from an
/// upstream model (schema-resolved via Salsa), ABS should return Decimal(10,2) not Double.
#[test]
fn abs_decimal_cross_model_schema_resolved() {
    use smelt_db::{typed_model_schema, Database, Workspace};
    use std::path::PathBuf;

    // Reproduce the multi-model proptest failure: upstream with multiple columns (including
    // INTERVAL type), downstream applies ABS to the Decimal column.
    // Key: the upstream has 4 columns and the downstream uses LEFT() on another column too.
    let upstream_sql = "WITH data AS (SELECT CAST('hello' AS STRING) AS str_col_1, CAST(99.99 AS DECIMAL(10,2)) AS dec_col_3) SELECT str_col_1, dec_col_3 FROM data";
    let downstream_sql =
        "SELECT ABS(dec_col_3) AS expr_0, LEFT(str_col_1, 3) AS expr_1 FROM smelt.models.upstream";

    let mut db = Database::default();
    let upstream_path = PathBuf::from("models/upstream.sql");
    let downstream_path = PathBuf::from("models/downstream.sql");
    let root = PathBuf::from(".");

    let upstream_file = db.set_source_file(
        upstream_path.clone(),
        upstream_sql.to_string(),
        root.clone(),
    );
    let downstream_file = db.set_source_file(
        downstream_path.clone(),
        downstream_sql.to_string(),
        root.clone(),
    );
    let project = db.set_project_input(root, String::new());
    db.set_workspace(vec![upstream_file, downstream_file], vec![project]);

    let ws = Workspace::get(&db);

    // First confirm the upstream schema has dec_col_3: Decimal(10,2)
    let upstream_schema = typed_model_schema(&db, ws, upstream_file);
    let dec_col = upstream_schema
        .columns
        .iter()
        .find(|c| c.name == "dec_col_3");
    assert!(
        dec_col.is_some(),
        "upstream schema should contain dec_col_3; got: {:?}",
        upstream_schema
            .columns
            .iter()
            .map(|c| &c.name)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        dec_col.unwrap().data_type.as_ref().map(|tc| &tc.data_type),
        Some(&DataType::Decimal {
            precision: 10,
            scale: 2
        }),
        "upstream dec_col_3 should be Decimal(10,2)"
    );

    // Now check the downstream inference: ABS(dec_col_3) should be Decimal(10,2)
    let downstream_schema = typed_model_schema(&db, ws, downstream_file);
    assert!(
        !downstream_schema.columns.is_empty(),
        "downstream schema should have columns"
    );
    let result_col = downstream_schema
        .columns
        .iter()
        .find(|c| c.name == "expr_0")
        .expect("should have expr_0 column");
    assert_eq!(
        result_col.data_type.as_ref().map(|tc| &tc.data_type),
        Some(&DataType::Decimal {
            precision: 10,
            scale: 2
        }),
        "ABS(dec_col_3) from upstream schema should yield Decimal(10,2); got: {:?}",
        result_col.data_type
    );
}
