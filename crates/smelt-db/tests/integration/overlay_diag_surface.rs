//! BUG-014 P4 regression: overlay validation diagnostics surface through
//! `file_diagnostics` when a build target is active.
//!
//! `overlay_diag_surfaces_unknown_field`: base is valid, overlay has an
//! unknown field → exactly one `ConfigLoaderUnknownField` fires for the
//! generator file that contains the `smelt.config.load_yaml(...)` call.

use smelt_db::{file_diagnostics, Database, DiagnosticCode, Workspace};
use std::sync::Arc;

const GEN_SRC: &str = r#"---
generates: models
---
smelt.config.load_yaml('cohorts.yaml', List<{ name: Text, min_revenue: Integer }>)
  |> map(fn c => ModelDef {
       name: c.name,
       body: SELECT 1 AS id
     })
"#;

const BASE_YAML: &str = "- name: west\n  min_revenue: 100\n";
const OVERLAY_INVALID_YAML: &str =
    "- name: west\n  min_revenue: 999\n  extra_field: this_is_unknown\n";

#[test]
fn overlay_diag_surfaces_unknown_field() {
    let project_root = std::path::PathBuf::from("/overlay_diag_test");
    let mut db = Database::default();

    let project = db.set_project_input(project_root.clone(), "".to_string());
    let gen_sf = db.set_source_file(
        project_root.join("models/cohorts.gen.sql"),
        GEN_SRC.to_string(),
        project_root.clone(),
    );
    db.set_workspace(vec![gen_sf], vec![project]);

    // Base file: valid.
    db.set_loader_file(Arc::from("cohorts.yaml"), Arc::from(BASE_YAML), true);
    // Overlay file: unknown field → ConfigLoaderUnknownField expected.
    db.set_loader_file(
        Arc::from("cohorts.prod.yaml"),
        Arc::from(OVERLAY_INVALID_YAML),
        true,
    );

    // Activate the "prod" target.
    db.set_active_target(Some(Arc::from("prod")));

    let ws = Workspace::try_get(&db).expect("workspace should be initialized");
    let diags = file_diagnostics(&db, ws, gen_sf);

    let overlay_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ConfigLoaderUnknownField))
        .collect();

    assert_eq!(
        overlay_diags.len(),
        1,
        "expected exactly 1 ConfigLoaderUnknownField from overlay, got {}.\nAll diags:\n{:#?}",
        overlay_diags.len(),
        diags
    );
}

/// When no target is active, no overlay diagnostics are emitted (even if the
/// overlay file exists and is invalid).
#[test]
fn overlay_diag_absent_when_no_target() {
    let project_root = std::path::PathBuf::from("/overlay_diag_no_target");
    let mut db = Database::default();

    let project = db.set_project_input(project_root.clone(), "".to_string());
    let gen_sf = db.set_source_file(
        project_root.join("models/cohorts.gen.sql"),
        GEN_SRC.to_string(),
        project_root.clone(),
    );
    db.set_workspace(vec![gen_sf], vec![project]);

    db.set_loader_file(Arc::from("cohorts.yaml"), Arc::from(BASE_YAML), true);
    db.set_loader_file(
        Arc::from("cohorts.prod.yaml"),
        Arc::from(OVERLAY_INVALID_YAML),
        true,
    );
    // No active_target set → overlay is not consulted.

    let ws = Workspace::try_get(&db).expect("workspace should be initialized");
    let diags = file_diagnostics(&db, ws, gen_sf);

    let overlay_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ConfigLoaderUnknownField))
        .collect();

    assert_eq!(
        overlay_diags.len(),
        0,
        "expected 0 overlay diagnostics when no target is active, got {}.\nAll diags:\n{:#?}",
        overlay_diags.len(),
        diags
    );
}
