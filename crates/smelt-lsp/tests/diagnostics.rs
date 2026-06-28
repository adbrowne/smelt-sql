//! Phase 2c — kind-mismatch diagnostic test.
//!
//! Test 7: `test_path_in_tableexpr_position_diagnostic`
//!   Using `smelt.tests.foo` in a FROM (TableExpr) position must emit a
//!   `DiagnosticCode::KindMismatch` diagnostic.
//!
//! This exercises the already-wired Phase 2a code path; this test pins the
//! contract so regressions are caught.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

// ---------------------------------------------------------------------------
// Workspace helper
// ---------------------------------------------------------------------------

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let mut handles = Vec::new();
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), content.to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

// ---------------------------------------------------------------------------
// Test 7: test_path_in_tableexpr_position_diagnostic
// ---------------------------------------------------------------------------

/// A model that uses `smelt.tests.foo` in a FROM clause must receive a
/// `KindMismatch` diagnostic because test entities cannot appear in
/// `TableExpr` position.
#[test]
fn test_path_in_tableexpr_position_diagnostic() {
    let root = PathBuf::from("/fake/diag_project");

    // A test file (identified by a `smelt.test` declaration).
    let test_path = root.join("tests").join("foo.sql");
    let test_src = "smelt.test foo AS (SELECT 1 AS x WHERE 1 = 0)\nEXPECT ({x: 1})\n";

    // A model that incorrectly uses the test in a FROM position.
    let model_path = root.join("models").join("bad_from.sql");
    let model_src = "SELECT * FROM smelt.tests.foo\n";

    let (db, ws, files) = build_db(
        root,
        &[(test_path, test_src), (model_path.clone(), model_src)],
    );

    let model_file = files[1];
    let diags = file_diagnostics(&db, ws, model_file);

    let kind_mismatches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::KindMismatch))
        .collect();

    assert!(
        !kind_mismatches.is_empty(),
        "expected KindMismatch diagnostic for `FROM smelt.tests.foo`; \
         got diagnostics: {diags:#?}"
    );
}

/// Sanity: a model referencing a model-kind path in FROM must NOT emit
/// KindMismatch.
#[test]
fn no_kind_mismatch_for_model_in_tableexpr() {
    let root = PathBuf::from("/fake/diag_ok_project");

    let users_path = root.join("models").join("users.sql");
    let users_src = "SELECT 1 AS id\n";
    let caller_path = root.join("models").join("caller.sql");
    let caller_src = "SELECT * FROM smelt.models.users\n";

    let (db, ws, files) = build_db(root, &[(users_path, users_src), (caller_path, caller_src)]);

    let caller_file = files[1];
    let diags = file_diagnostics(&db, ws, caller_file);

    let kind_mismatches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::KindMismatch))
        .collect();

    assert!(
        kind_mismatches.is_empty(),
        "model-kind path in FROM should NOT emit KindMismatch; \
         got: {kind_mismatches:#?}"
    );
}
