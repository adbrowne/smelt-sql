//! Phase 7 — missing-sidecar diagnostic tests.
//!
//! Test: `csv_without_sibling_yml_warns`
//!   A CSV file registered in the workspace without a sibling `.yml` must
//!   emit exactly one `DiagnosticCode::MissingSeedSidecar` warning.
//!
//! Test: `adding_sibling_yml_clears_warning`
//!   Once a sibling `.yml` is created on disk and the file is re-scanned,
//!   the warning disappears.

use std::path::PathBuf;

use smelt_db::{
    file_diagnostics, Database, DiagnosticCode, DiagnosticSeverity, SourceFile, Workspace,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Workspace helpers
// ---------------------------------------------------------------------------

fn build_db_with_csv(
    project_root: PathBuf,
    csv_path: PathBuf,
    csv_content: &str,
) -> (Database, Workspace, SourceFile) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let sf = db.set_source_file(csv_path, csv_content.to_string(), project_root);
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();
    (db, ws, sf)
}

// ---------------------------------------------------------------------------
// Test: csv_without_sibling_yml_warns
// ---------------------------------------------------------------------------

/// A CSV file without a sibling `.yml` must emit exactly one
/// `MissingSeedSidecar` warning with the specified message fragment.
#[test]
fn csv_without_sibling_yml_warns() {
    let tmp = TempDir::new().unwrap();
    let seeds_dir = tmp.path().join("seeds");
    std::fs::create_dir_all(&seeds_dir).unwrap();

    // Write the CSV to disk (no sibling .yml).
    let csv_path = seeds_dir.join("orders.csv");
    let csv_content = "id,amount\n1,100\n2,200\n";
    std::fs::write(&csv_path, csv_content).unwrap();

    let (db, ws, sf) = build_db_with_csv(tmp.path().to_path_buf(), csv_path, csv_content);

    let diags = file_diagnostics(&db, ws, sf);

    let sidecar_warns: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MissingSeedSidecar))
        .collect();

    assert!(
        !sidecar_warns.is_empty(),
        "expected MissingSeedSidecar warning for CSV without sibling .yml; \
         got diagnostics: {diags:#?}"
    );

    assert_eq!(
        sidecar_warns.len(),
        1,
        "expected exactly one MissingSeedSidecar warning; got: {sidecar_warns:#?}"
    );

    let warn = sidecar_warns[0];
    assert_eq!(
        warn.severity,
        DiagnosticSeverity::Warning,
        "MissingSeedSidecar must be Warning severity"
    );

    assert!(
        warn.message.contains("inferred") && warn.message.contains("pin"),
        "warning message should mention inference and pinning; got: {:?}",
        warn.message
    );
}

// ---------------------------------------------------------------------------
// Test: adding_sibling_yml_clears_warning
// ---------------------------------------------------------------------------

/// Once a sibling `.yml` exists on disk, re-checking the CSV file must
/// produce no `MissingSeedSidecar` diagnostic.
#[test]
fn adding_sibling_yml_clears_warning() {
    let tmp = TempDir::new().unwrap();
    let seeds_dir = tmp.path().join("seeds");
    std::fs::create_dir_all(&seeds_dir).unwrap();

    let csv_path = seeds_dir.join("products.csv");
    let csv_content = "sku,price\nA001,9.99\n";
    std::fs::write(&csv_path, csv_content).unwrap();

    // Phase 1: no sidecar — should warn.
    {
        let (db, ws, sf) =
            build_db_with_csv(tmp.path().to_path_buf(), csv_path.clone(), csv_content);
        let diags = file_diagnostics(&db, ws, sf);
        let count = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::MissingSeedSidecar))
            .count();
        assert!(count > 0, "precondition: should warn before sidecar exists");
    }

    // Write the sibling sidecar.
    let yml_path = seeds_dir.join("products.yml");
    std::fs::write(
        &yml_path,
        "columns:\n  - name: sku\n    type: TEXT\n  - name: price\n    type: DOUBLE\n",
    )
    .unwrap();

    // Phase 2: sidecar now exists — warning must disappear.
    {
        let (db, ws, sf) =
            build_db_with_csv(tmp.path().to_path_buf(), csv_path.clone(), csv_content);
        let diags = file_diagnostics(&db, ws, sf);
        let count = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::MissingSeedSidecar))
            .count();
        assert_eq!(
            count, 0,
            "MissingSeedSidecar warning should disappear once sidecar exists; \
             got diagnostics: {diags:#?}"
        );
    }
}
