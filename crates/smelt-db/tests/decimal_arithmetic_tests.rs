//! Phase 1+2+3: Decimal arithmetic type inference tests.
//!
//! Phase 1 (already passing): `DecimalPrecisionOverflow` diagnostic code exists.
//! Phase 2 (TDD): Growth formulas for Decimal arithmetic, integer lifting,
//! and overflow detection.
//! Phase 3 (TDD): Decimal division rejection — emits TypeMismatch and returns Unknown.

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
