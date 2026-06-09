//! Phase 6 swallowed-errors triage — type-inference + LSP layers.
//!
//! ## Triage summary
//!
//! Every `let _ =`, `.ok()`, and `Err(_) =>` site in `smelt-db/src` and
//! `smelt-lsp/src` was audited.  Results:
//!
//! **Genuinely reportable failures — NONE found.**  All sites fall into one of:
//!   - Already covered by a parallel diagnostic path.
//!   - Legitimately best-effort (cache I/O, optional enrichment).
//!   - Conservative type inference returning `Unknown` (the check layer catches).
//!   - Filtering / short-circuit where the `Err` is structurally inapplicable.
//!
//! Each site now carries `// intentionally ignored: <reason>` in its source file.
//!
//! **needs-review finding (BUG-078):**
//! `sources_yaml_error` in `smelt-db/src/lib.rs:1753` is dead code — it is
//! guarded by `if !sources.is_empty()` but `model_sources` always returns an
//! empty Vec (the `smelt.source()` migration stub).  A malformed aggregate
//! `sources.yml` therefore produces no diagnostic, contradicting the sources
//! spec §Constraint 6 ("its presence produces a clear migration error").
//! Logged in `docs/bug-hunt/2026-05-30-findings.md`.
//!
//! ## Regression guards
//!
//! The tests below verify that the existing diagnostic coverage for the cases
//! that *look* like swallowed errors (but are correctly handled through a
//! parallel path) has not regressed.

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

/// `infer_cast_type` in `literal.rs` calls `parse_type(&type_text).ok()?`,
/// silently returning `None` when the type name is unrecognised.  This would
/// be a swallowed error, but `collect_expression_type_diagnostics` in
/// `check_types.rs` independently calls `parse_type` and emits
/// `UnknownCastType` — so the failure IS reported.
///
/// This test is the regression guard: if someone removes the
/// `check_types.rs` CAST check, this test turns red.
#[test]
fn cast_to_unrecognized_type_emits_diagnostic_not_silently_unknown() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("bad_cast.sql");
    let src = "SELECT CAST(1 AS NotAType) AS x";

    let (db, ws, handles) = build_db(&[(model_path, src)]);
    let model_file = handles[0];

    let diags = file_diagnostics(&db, ws, model_file);
    let cast_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownCastType))
        .collect();

    assert_eq!(
        cast_diags.len(),
        1,
        "CAST to an unrecognised type must emit exactly one UnknownCastType \
         diagnostic (coverage via check_types.rs, not infer_cast_type); got {diags:#?}"
    );
}

/// Valid CAST expressions must NOT emit `UnknownCastType`.
#[test]
fn cast_to_recognised_type_no_diagnostic() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("good_cast.sql");
    let src = "SELECT CAST(1 AS BIGINT) AS x, CAST('2024-01-01' AS DATE) AS d";

    let (db, ws, handles) = build_db(&[(model_path, src)]);
    let model_file = handles[0];

    let diags = file_diagnostics(&db, ws, model_file);
    let cast_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownCastType))
        .collect();

    assert!(
        cast_diags.is_empty(),
        "valid CAST expressions should produce no UnknownCastType; got {cast_diags:#?}"
    );
}
