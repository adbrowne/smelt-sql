//! Tests for P2: model-level state posture widening rejection (D-47).
//!
//! A model may narrow but not widen the project's `state.mode` posture.
//! Widening must emit a `StateModeWidening` diagnostic.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

fn build_db_with_smelt_yml(
    smelt_yml: &str,
    model_path: PathBuf,
    model_sql: &str,
) -> (Database, Workspace, SourceFile) {
    let root = PathBuf::from("/fake/project");
    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    db.set_project_smelt_yml(&root, smelt_yml.to_string());
    let sf = db.set_source_file(model_path.clone(), model_sql.to_string(), root.clone());
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();
    (db, ws, sf)
}

/// Model declares `state: {mode: environments}` in a `stateless` project →
/// StateModeWidening diagnostic (widening is rejected).
#[test]
fn model_widening_to_environments_in_stateless_project_is_rejected() {
    let smelt_yml = "name: test\nversion: 1\n# no state: block → defaults to stateless\n";
    let model_path = PathBuf::from("/fake/project/models/my_model.sql");
    let model_sql = "---\nstate:\n  mode: environments\n---\nSELECT 1 AS x";

    let (db, ws, sf) = build_db_with_smelt_yml(smelt_yml, model_path, model_sql);
    let diags = file_diagnostics(&db, ws, sf);

    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::StateModeWidening)),
        "expected StateModeWidening diagnostic; got: {diags:#?}"
    );
}

/// Model declares `state: {mode: intervals}` in a `stateless` project →
/// StateModeWidening diagnostic.
#[test]
fn model_widening_to_intervals_in_stateless_project_is_rejected() {
    let smelt_yml = "name: test\nversion: 1\n";
    let model_path = PathBuf::from("/fake/project/models/my_model.sql");
    let model_sql = "---\nstate:\n  mode: intervals\n---\nSELECT 1 AS x";

    let (db, ws, sf) = build_db_with_smelt_yml(smelt_yml, model_path, model_sql);
    let diags = file_diagnostics(&db, ws, sf);

    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::StateModeWidening)),
        "expected StateModeWidening diagnostic; got: {diags:#?}"
    );
}

/// Model declares `state: {mode: stateless}` in an `environments` project →
/// allowed (narrowing is fine).
#[test]
fn model_narrowing_to_stateless_in_environments_project_is_allowed() {
    let smelt_yml = "name: test\nversion: 1\nstate:\n  mode: environments\n";
    let model_path = PathBuf::from("/fake/project/models/my_model.sql");
    let model_sql = "---\nstate:\n  mode: stateless\n---\nSELECT 1 AS x";

    let (db, ws, sf) = build_db_with_smelt_yml(smelt_yml, model_path, model_sql);
    let diags = file_diagnostics(&db, ws, sf);

    let widening_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::StateModeWidening))
        .collect();
    assert!(
        widening_diags.is_empty(),
        "model narrowing to stateless must be allowed; got: {widening_diags:#?}"
    );
}

/// Model declares `state: {mode: environments}` in an `environments` project →
/// same posture, no widening, allowed.
#[test]
fn model_same_mode_as_project_is_allowed() {
    let smelt_yml = "name: test\nversion: 1\nstate:\n  mode: environments\n";
    let model_path = PathBuf::from("/fake/project/models/my_model.sql");
    let model_sql = "---\nstate:\n  mode: environments\n---\nSELECT 1 AS x";

    let (db, ws, sf) = build_db_with_smelt_yml(smelt_yml, model_path, model_sql);
    let diags = file_diagnostics(&db, ws, sf);

    let widening_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::StateModeWidening))
        .collect();
    assert!(
        widening_diags.is_empty(),
        "model declaring same mode as project must be allowed; got: {widening_diags:#?}"
    );
}

/// Model has no `state:` frontmatter → no widening diagnostic regardless
/// of project posture.
#[test]
fn model_without_state_field_never_widens() {
    let smelt_yml = "name: test\nversion: 1\nstate:\n  mode: environments\n";
    let model_path = PathBuf::from("/fake/project/models/my_model.sql");
    let model_sql = "---\nname: my_model\n---\nSELECT 1 AS x";

    let (db, ws, sf) = build_db_with_smelt_yml(smelt_yml, model_path, model_sql);
    let diags = file_diagnostics(&db, ws, sf);

    let widening_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::StateModeWidening))
        .collect();
    assert!(
        widening_diags.is_empty(),
        "absent state field must not produce widening diagnostic; got: {widening_diags:#?}"
    );
}
