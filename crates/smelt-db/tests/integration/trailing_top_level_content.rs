//! Fail-loud trailing top-level content — a model file parses at most one
//! query body; any top-level content left over afterwards must surface as a
//! `TrailingTopLevelContent` diagnostic instead of being silently absorbed.
//!
//! See `docs/specs/diagnostics.md` §"Fail-loud invariants" #3 and the
//! `TrailingTopLevelContent` catalogue entry.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

fn build_db(
    project_root: PathBuf,
    smelt_yml: &str,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    db.set_project_smelt_yml(&project_root, smelt_yml.to_string());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

const SMELT_YML: &str = "name: trailing_top_level_content_test
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    schema: main
default_materialization: view
";

/// The `examples/broken/models/trailing_top_level_content.sql` fixture: a
/// second top-level `SELECT` after the model body. Must produce a
/// `TrailingTopLevelContent` diagnostic whose range covers the junk, not a
/// silent zero-diagnostic parse.
#[test]
fn trailing_content_surfaces_as_diagnostic() {
    let project_root = PathBuf::from("/project");
    let model_path = project_root.join("models/trailing_top_level_content.sql");
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples/broken/models/trailing_top_level_content.sql"),
    )
    .expect("fixture must exist");

    let (db, ws, handles) = build_db(project_root, SMELT_YML, &[(model_path, &text)]);
    let file = handles[0];

    let diags = file_diagnostics(&db, ws, file);
    let trailing: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::TrailingTopLevelContent))
        .collect();

    assert_eq!(
        trailing.len(),
        1,
        "expected exactly one TrailingTopLevelContent diagnostic, got: {:?}",
        diags
    );

    // The range must cover the junk (the second SELECT), not be empty at the
    // start of the file.
    let junk_offset = text.find("\nSELECT id FROM orders").expect("fixture shape") as u32 + 1;
    assert!(
        u32::from(trailing[0].range.start()) >= junk_offset,
        "TrailingTopLevelContent range must cover the junk content, got range {:?} \
         (junk starts at byte {junk_offset})",
        trailing[0].range
    );
}

/// A clean single-query model must not trip the trailing-content check.
#[test]
fn clean_model_has_no_trailing_content_diagnostic() {
    let project_root = PathBuf::from("/project");
    let model_path = project_root.join("models/clean.sql");
    let text = "SELECT id FROM users";

    let (db, ws, handles) = build_db(project_root, SMELT_YML, &[(model_path, text)]);
    let file = handles[0];

    let diags = file_diagnostics(&db, ws, file);
    let trailing: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::TrailingTopLevelContent))
        .collect();
    assert!(
        trailing.is_empty(),
        "expected zero TrailingTopLevelContent diagnostics for a clean model, got: {:?}",
        trailing
    );
}
