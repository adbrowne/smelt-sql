use smelt_db::diagnostics_types::DiagnosticCode;

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
