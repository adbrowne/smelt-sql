//! Phase 1 collation tests (§17 of types.md).
//!
//! TDD tests — written before implementation to drive the red-green cycle.
//!
//! Three test groups:
//! 1. `non_portable_collation_code_exists` — compile-time gate: the variant exists.
//! 2. `binary_collation_passes_through` — binary collation names infer the operand
//!    type unchanged and emit no `NonPortableCollation` diagnostic.
//! 3. `non_binary_collation_diagnoses` — non-binary collation emits one
//!    `NonPortableCollation` Error anchored at the COLLATE clause span; the
//!    expression type degrades to `DataType::Unknown`.

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
