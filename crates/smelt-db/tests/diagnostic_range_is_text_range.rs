//! Compile-time and runtime smoke tests asserting that `Diagnostic::range` is
//! `rowan::TextRange`, not the legacy `smelt_parser::ast::Range`.
//!
//! Running this file (even just compiling it) is the guarantee that the type
//! flip in Phase 2 took effect. No test here should be skipped.

use rowan::TextRange;
use smelt_db::{Diagnostic, DiagnosticCode, DiagnosticSeverity};

/// A `Diagnostic` can be constructed with a `TextRange` as the `range` field.
/// If `range` were still `smelt_parser::ast::Range` this would fail to compile.
#[test]
fn diagnostic_range_accepts_text_range() {
    let tr = TextRange::new(0.into(), 5.into());
    let _diag = Diagnostic {
        severity: DiagnosticSeverity::Error,
        message: "test".to_string(),
        range: tr,
        code: Some(DiagnosticCode::ParseError),
        data: None,
    };
    // Structural check: the range field is a TextRange.
    let retrieved: TextRange = _diag.range;
    assert_eq!(retrieved, tr);
}

/// The start and end of the range are `TextSize` values (rowan), not
/// `smelt_parser::ast::Position` structs. Accessing `.start()` and `.end()`
/// on a `TextRange` is the distinguishing API.
#[test]
fn diagnostic_range_start_end_are_text_size() {
    let tr = TextRange::new(10.into(), 20.into());
    let diag = Diagnostic {
        severity: DiagnosticSeverity::Warning,
        message: "range check".to_string(),
        range: tr,
        code: None,
        data: None,
    };
    // These only compile if range is TextRange.
    let _start: rowan::TextSize = diag.range.start();
    let _end: rowan::TextSize = diag.range.end();
    assert_eq!(u32::from(diag.range.start()), 10);
    assert_eq!(u32::from(diag.range.end()), 20);
}
