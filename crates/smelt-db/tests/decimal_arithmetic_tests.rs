//! Phase 1+2: Decimal arithmetic type inference tests.
//!
//! Phase 1 (already passing): `DecimalPrecisionOverflow` diagnostic code exists.
//! Phase 2 (TDD): Growth formulas for Decimal arithmetic, integer lifting,
//! and overflow detection.

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
