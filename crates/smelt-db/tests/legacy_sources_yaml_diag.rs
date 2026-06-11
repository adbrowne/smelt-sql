//! BUG-078: a malformed aggregate `sources.yml` must surface a diagnostic.
//!
//! The `sources_yaml_error` consumer in `file_diagnostics` was dead code —
//! gated behind `!sources.is_empty()`, where `sources` (legacy `smelt.source()`
//! call sites) is always empty since the per-entity sources migration, so a
//! project with a YAML-broken aggregate `sources.yml` silently fell back to
//! `SourcesConfig::default()` with no diagnostic.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode};

#[test]
fn malformed_aggregate_sources_yml_emits_yaml_parse_error() {
    let root = PathBuf::from("/test_project");
    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), "sources: [unclosed".to_string());
    let sf = db.set_source_file(
        root.join("models/m.sql"),
        "SELECT 1 AS x".to_string(),
        root.clone(),
    );
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();

    let diags: Vec<_> = file_diagnostics(&db, ws, sf)
        .into_iter()
        .filter(|d| d.code == Some(DiagnosticCode::YamlParseError))
        .collect();
    assert_eq!(
        diags.len(),
        1,
        "a malformed aggregate sources.yml must produce a YamlParseError \
         diagnostic, got: {diags:?}"
    );
}

#[test]
fn valid_aggregate_sources_yml_emits_no_yaml_parse_error() {
    let root = PathBuf::from("/test_project");
    let mut db = Database::default();
    let project = db.set_project_input(
        root.clone(),
        "sources:\n  raw:\n    tables:\n      users:\n        columns:\n          - name: id\n            type: INTEGER\n".to_string(),
    );
    let sf = db.set_source_file(
        root.join("models/m.sql"),
        "SELECT 1 AS x".to_string(),
        root.clone(),
    );
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();

    let diags: Vec<_> = file_diagnostics(&db, ws, sf)
        .into_iter()
        .filter(|d| d.code == Some(DiagnosticCode::YamlParseError))
        .collect();
    assert!(
        diags.is_empty(),
        "a well-formed aggregate sources.yml must not produce YamlParseError: {diags:?}"
    );
}
