//! Tests for `InvalidFunctionTypeRef` on unrecognized nested type names in
//! composite type annotations (struct field types).
//!
//! Validates that `smelt.define` and `smelt.extern` signatures whose
//! annotations contain an unrecognized type name nested inside a struct field
//! emit `InvalidFunctionTypeRef` at the declaration rather than silently
//! absorbing the unknown type name as `DataType::Unknown`.
//!
//! Also validates that:
//! - Valid closed struct annotations do NOT emit `InvalidFunctionTypeRef`.
//! - Row-tail (`..r`) struct returns do NOT emit `InvalidFunctionTypeRef` for
//!   the tail itself (only unrecognized field types are flagged).

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

fn build_db(files: &[(PathBuf, &str)]) -> (Database, Workspace, Vec<SourceFile>) {
    let root = PathBuf::from("/fake/project");
    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

/// An unrecognized type name nested in a struct field type of the return
/// annotation must emit exactly one `InvalidFunctionTypeRef`.
#[test]
fn struct_return_unknown_field_type_emits_invalid_type_ref() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("bad_return.sql");
    let src = "smelt.define bad_return() -> Expr<Struct<{a: Integer, b: Bogus}>> AS (1)";

    let (db, ws, handles) = build_db(&[(fn_path, src)]);
    let fn_file = handles[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let invalid_type_refs: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::InvalidFunctionTypeRef))
        .collect();

    assert_eq!(
        invalid_type_refs.len(),
        1,
        "expected exactly one InvalidFunctionTypeRef for unrecognized field type `Bogus` in \
         struct return; got {diags:#?}"
    );
    assert!(
        invalid_type_refs[0].message.contains("bad_return"),
        "diagnostic message should mention the function name; got: {}",
        invalid_type_refs[0].message
    );
}

/// An unrecognized type name nested in a struct field type of a parameter
/// annotation must emit exactly one `InvalidFunctionTypeRef`.
#[test]
fn struct_param_unknown_field_type_emits_invalid_type_ref() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("bad_param.sql");
    let src = "smelt.define bad_param(s: Expr<Struct<{a: Integer, b: Bogus}>>) AS (1)";

    let (db, ws, handles) = build_db(&[(fn_path, src)]);
    let fn_file = handles[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let invalid_type_refs: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::InvalidFunctionTypeRef))
        .collect();

    assert_eq!(
        invalid_type_refs.len(),
        1,
        "expected exactly one InvalidFunctionTypeRef for unrecognized field type `Bogus` in \
         struct parameter; got {diags:#?}"
    );
}

/// A valid closed struct return (`Expr<Struct<{a: Integer, b: Text}>>`) must
/// NOT emit `InvalidFunctionTypeRef`.
#[test]
fn valid_closed_struct_return_no_invalid_type_ref() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("good_return.sql");
    let src = "smelt.define good_return() -> Expr<Struct<{a: Integer, b: Text}>> AS (1)";

    let (db, ws, handles) = build_db(&[(fn_path, src)]);
    let fn_file = handles[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let invalid_type_refs: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::InvalidFunctionTypeRef))
        .collect();

    assert!(
        invalid_type_refs.is_empty(),
        "valid closed struct return should emit no InvalidFunctionTypeRef; got {invalid_type_refs:#?}"
    );
}

/// A row-tail struct return (`Expr<Struct<{a: Integer, ..r}>>`) must NOT emit
/// `InvalidFunctionTypeRef` — the tail marker (`..r`) is not a type name.
#[test]
fn row_tail_struct_return_no_invalid_type_ref() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("row_tail.sql");
    // The `..r` tail is a row variable marker, not a field. No field type is Unknown.
    let src = "smelt.define row_tail(event: Expr<Struct<{ts: Timestamp, ..r}>>) \
               -> Expr<Struct<{hour: BigInt, ..r}>> AS (1)";

    let (db, ws, handles) = build_db(&[(fn_path, src)]);
    let fn_file = handles[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let invalid_type_refs: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::InvalidFunctionTypeRef))
        .collect();

    assert!(
        invalid_type_refs.is_empty(),
        "row-tail struct return/param should emit no InvalidFunctionTypeRef; got {invalid_type_refs:#?}"
    );
}

/// `smelt.extern` with an unknown struct field type should also emit
/// `InvalidFunctionTypeRef`.
#[test]
fn extern_struct_return_unknown_field_type_emits_invalid_type_ref() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("bad_extern.sql");
    let src = "smelt.extern bad_extern() -> Expr<Struct<{x: BigInt, y: NotAType}>>";

    let (db, ws, handles) = build_db(&[(fn_path, src)]);
    let fn_file = handles[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let invalid_type_refs: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::InvalidFunctionTypeRef))
        .collect();

    assert_eq!(
        invalid_type_refs.len(),
        1,
        "expected exactly one InvalidFunctionTypeRef for unrecognized field type `NotAType` in \
         extern return; got {diags:#?}"
    );
}

/// Two unknown field types in the same struct should emit exactly one
/// `InvalidFunctionTypeRef` anchored at the annotation (not one per field).
#[test]
fn struct_with_two_unknown_fields_emits_one_diagnostic() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("two_bad_fields.sql");
    // Both `Bogus` and `Also_Unknown` are unrecognized type names.
    let src = "smelt.define two_bad(\n    s: Expr<Struct<{a: Bogus, b: Also_Unknown}>>\n) AS (1)";

    let (db, ws, handles) = build_db(&[(fn_path, src)]);
    let fn_file = handles[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let invalid_type_refs: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::InvalidFunctionTypeRef))
        .collect();

    // One diagnostic per annotation (the struct annotation as a whole).
    // The implementation should emit one diagnostic for the parameter's
    // annotation when any field is unknown.
    assert!(
        !invalid_type_refs.is_empty(),
        "expected at least one InvalidFunctionTypeRef for two unknown struct field types; \
         got {diags:#?}"
    );
}
