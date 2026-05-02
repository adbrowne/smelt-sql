//! TB-5 — `smelt.<path>(...)` call-path-prefix enforcement.
//!
//! The spec says (`docs/specs/functions.md` §"Function call syntax"):
//!
//!   `<path>` is the workspace-relative directory of the declaring file
//!   joined with the function name. The filename stem is **not** a path
//!   component.
//!
//! These tests verify that calling a user-declared function with the wrong
//! path prefix (e.g. including the file stem, or naming a directory that
//! does not contain the declaring file) emits `UnknownSmeltFn`.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

fn diags_with_code(
    db: &Database,
    ws: Workspace,
    file: SourceFile,
    code: DiagnosticCode,
) -> Vec<smelt_db::Diagnostic> {
    file_diagnostics(db, ws, file)
        .into_iter()
        .filter(|d| d.code == Some(code))
        .collect()
}

const STATUS_FN_SRC: &str =
    "smelt.define is_shipped(status: Expr<Text>) -> Expr<Boolean> AS (status = 'shipped')\n";

/// Wrong-path prefix: the declaring file is `functions/status.sql`, so the
/// only valid call path is `smelt.functions.is_shipped(...)`. Calling
/// `smelt.functions.nonexistent.is_shipped(...)` must emit `UnknownSmeltFn`.
#[test]
fn test_unknown_smelt_fn_wrong_path_prefix() {
    let root = PathBuf::from("/fake/project/wrong_prefix");
    let fn_path = root.join("functions").join("status.sql");

    let model_path = root.join("models").join("call.sql");
    let model_src = "SELECT smelt.functions.nonexistent.is_shipped('shipped') AS r\n";

    let (db, ws, files) = build_db(
        root,
        &[(fn_path, STATUS_FN_SRC), (model_path.clone(), model_src)],
    );
    let model_file = files[1];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::UnknownSmeltFn);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one UnknownSmeltFn diagnostic for wrong path prefix, got {diags:?}"
    );
    assert!(
        diags[0]
            .message
            .contains("smelt.functions.nonexistent.is_shipped"),
        "diagnostic should name the wrong call path, got: {}",
        diags[0].message
    );
}

/// File-stem in path: the spec is explicit that the filename stem is not a
/// path component. `smelt.functions.status.is_shipped(...)` (where `status`
/// is the file stem) must emit `UnknownSmeltFn`.
#[test]
fn test_unknown_smelt_fn_file_stem_in_path() {
    let root = PathBuf::from("/fake/project/file_stem");
    let fn_path = root.join("functions").join("status.sql");

    let model_path = root.join("models").join("call.sql");
    let model_src = "SELECT smelt.functions.status.is_shipped('shipped') AS r\n";

    let (db, ws, files) = build_db(
        root,
        &[(fn_path, STATUS_FN_SRC), (model_path.clone(), model_src)],
    );
    let model_file = files[1];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::UnknownSmeltFn);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one UnknownSmeltFn diagnostic for file-stem-in-path, got {diags:?}"
    );
    assert!(
        diags[0]
            .message
            .contains("smelt.functions.status.is_shipped"),
        "diagnostic should name the wrong call path, got: {}",
        diags[0].message
    );
}

/// Correct path: `smelt.functions.is_shipped(...)` matches the declaring
/// file's workspace-relative directory `functions/`. No `UnknownSmeltFn`.
#[test]
fn test_known_smelt_fn_correct_path() {
    let root = PathBuf::from("/fake/project/correct_path");
    let fn_path = root.join("functions").join("status.sql");

    let model_path = root.join("models").join("call.sql");
    let model_src = "SELECT smelt.functions.is_shipped('shipped') AS r\n";

    let (db, ws, files) = build_db(
        root,
        &[(fn_path, STATUS_FN_SRC), (model_path.clone(), model_src)],
    );
    let model_file = files[1];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::UnknownSmeltFn);
    assert!(
        diags.is_empty(),
        "spec-correct call path must NOT emit UnknownSmeltFn, got {diags:?}"
    );
}

/// Existing `UnknownSmeltFn` behaviour for unknown leaf names must not
/// regress: a name that isn't declared anywhere still emits `UnknownSmeltFn`.
#[test]
fn test_unknown_smelt_fn_name_not_declared() {
    let root = PathBuf::from("/fake/project/undeclared_name");
    let fn_path = root.join("functions").join("status.sql");

    let model_path = root.join("models").join("call.sql");
    let model_src = "SELECT smelt.functions.totally_made_up('shipped') AS r\n";

    let (db, ws, files) = build_db(
        root,
        &[(fn_path, STATUS_FN_SRC), (model_path.clone(), model_src)],
    );
    let model_file = files[1];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::UnknownSmeltFn);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one UnknownSmeltFn diagnostic for undeclared name, got {diags:?}"
    );
    assert!(
        diags[0].message.contains("totally_made_up"),
        "diagnostic should name the unknown function, got: {}",
        diags[0].message
    );
}
