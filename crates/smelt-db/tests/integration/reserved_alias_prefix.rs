//! A user-written top-level SELECT-item alias beginning with the reserved
//! `_smelt_` prefix emits `ReservedProjectionAliasPrefix` via the production
//! `check_file_diagnostics` / `file_diagnostics` pipeline — the same surface
//! both `smelt-lsp` (`db_helpers::…` reads `file_diagnostics` directly) and
//! `smelt-cli`'s pre-execution gate (`commands::build`) consume, so a single
//! test here exercises both consumers per the diagnostic-parity rule
//! (`architecture.md` §"Diagnostic parity rule"). See
//! `docs/specs/multi_backend.md` §"Output-schema type conformance".

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

/// A `SELECT ... AS _smelt_foo` alias fires exactly one
/// `ReservedProjectionAliasPrefix` diagnostic.
#[test]
fn reserved_prefix_alias_emits_diagnostic() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("bad_alias.sql");
    let model_src = "SELECT 1 AS _smelt_foo FROM t\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src)]);
    let model_file = files[0];

    let diags: Vec<_> = file_diagnostics(&db, ws, model_file)
        .into_iter()
        .filter(|d| d.code == Some(DiagnosticCode::ReservedProjectionAliasPrefix))
        .collect();
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one ReservedProjectionAliasPrefix diagnostic for \
         `AS _smelt_foo`, got {diags:?}"
    );
}

/// An ordinary alias produces zero `ReservedProjectionAliasPrefix`
/// diagnostics — the check must not over-fire on unrelated names.
#[test]
fn ordinary_alias_no_diagnostic() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("ok_alias.sql");
    let model_src = "SELECT 1 AS foo FROM t\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src)]);
    let model_file = files[0];

    let diags: Vec<_> = file_diagnostics(&db, ws, model_file)
        .into_iter()
        .filter(|d| d.code == Some(DiagnosticCode::ReservedProjectionAliasPrefix))
        .collect();
    assert!(
        diags.is_empty(),
        "an ordinary alias must not trigger ReservedProjectionAliasPrefix: {diags:?}"
    );
}
