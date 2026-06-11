//! Phase 3 of silent-failures-hardening: unparseable struct field types must
//! emit `UnknownStructFieldType` at the individual field's range instead of
//! silently falling back to `DataType::Unknown`.

use std::path::PathBuf;

use smelt_db::{
    file_diagnostics, Database, DiagnosticCode, DiagnosticSeverity, SourceFile, Workspace,
};

const SMELT_YML: &str = "name: test
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    schema: main
default_materialization: view
";

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    db.set_project_smelt_yml(&project_root, SMELT_YML.to_string());
    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

/// A `smelt.define` parameter whose struct annotation contains an unrecognised
/// field type (`Bogus`) must emit `UnknownStructFieldType` with Error severity.
#[test]
fn unparseable_struct_field_emits_diagnostic() {
    let root = PathBuf::from("/fake/struct_field_type");
    let path = root.join("models").join("bad_struct.sql");
    let src = "\
smelt.define my_fn(t: Expr<Struct<{a: Integer, b: Bogus}>>) -> Expr<Integer> AS (
  t.a
)
";
    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];
    let diags = file_diagnostics(&db, ws, file);
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::UnknownStructFieldType)
                && d.severity == DiagnosticSeverity::Error),
        "expected Error-severity UnknownStructFieldType diagnostic; got {diags:#?}",
    );
}

/// A `smelt.define` with all-valid struct field types must NOT emit
/// `UnknownStructFieldType`.
#[test]
fn valid_struct_field_types_emit_no_unknown_diagnostic() {
    let root = PathBuf::from("/fake/struct_field_type_valid");
    let path = root.join("models").join("good_struct.sql");
    let src = "\
smelt.define my_fn(t: Expr<Struct<{a: Integer, b: Text}>>) -> Expr<Integer> AS (
  t.a
)
";
    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];
    let diags = file_diagnostics(&db, ws, file);
    assert!(
        diags
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::UnknownStructFieldType)),
        "expected no UnknownStructFieldType diagnostic on a valid struct annotation; got {diags:#?}",
    );
}

/// A `smelt.define` return type with an unrecognised struct field type also
/// emits `UnknownStructFieldType`.
#[test]
fn unparseable_struct_field_in_return_type_emits_diagnostic() {
    let root = PathBuf::from("/fake/struct_field_type_return");
    let path = root.join("models").join("bad_return_struct.sql");
    let src = "\
smelt.define my_fn() -> Expr<Struct<{a: Integer, b: NotAType}>> AS (
  {a: 1, b: 'x'}
)
";
    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];
    let diags = file_diagnostics(&db, ws, file);
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::UnknownStructFieldType)),
        "expected UnknownStructFieldType for bad return type struct field; got {diags:#?}",
    );
}
