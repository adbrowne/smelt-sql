//! Phase 52 — TDD tests for the two new lints:
//!   (a) MissingProvenancePushdownAdvisory (Hint) — fired when a transparent
//!       function in a FROM clause lacks declared provenance and a WHERE clause
//!       sits above the call.
//!   (b) ExternFragmentParamUnsupported (Error) — fired when a `smelt.extern`
//!       declaration uses a fragment-sort parameter type (`SelectItems`,
//!       `OrderSpec`).

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, Workspace};

// ============================================================================
// Helper: multi-file DB with unstable_schema: true
// ============================================================================

fn make_db_with_files(
    files: &[(&str, &str)],
    yml: &str,
) -> (Database, Workspace, Vec<smelt_db::SourceFile>) {
    let project_root = PathBuf::from("/tmp/phase52_test");
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    db.set_project_smelt_yml(&project_root, yml.to_string());
    let mut handles = Vec::new();
    for (name, content) in files {
        let path = project_root.join(name);
        let sf = db.set_source_file(path, content.to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = Workspace::try_get(&db).expect("workspace");
    (db, ws, handles)
}

fn get_diagnostics_single(content: &str) -> Vec<smelt_db::Diagnostic> {
    let project_root = PathBuf::from("/tmp/phase52_single_diag");
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let path = project_root.join("test_file.sql");
    let sf = db.set_source_file(path, content.to_string(), project_root.clone());
    db.set_workspace(vec![sf], vec![project]);
    let ws = Workspace::try_get(&db).expect("workspace");
    file_diagnostics(&db, ws, sf)
}

// ============================================================================
// Part (a): MissingProvenancePushdownAdvisory
// ============================================================================

/// Test 1 — function without provenance + WHERE → Hint fires.
#[test]
fn missing_provenance_pushdown_advisory() {
    let fn_file = "smelt.define my_func(src: TableExpr) -> TableExpr AS (SELECT * FROM src)";
    let model_file = "SELECT * FROM smelt.fn.my_func(smelt.ref('tbl')) WHERE total > 100";

    let (db, ws, handles) = make_db_with_files(
        &[
            ("functions/my_func.sql", fn_file),
            ("models/model.sql", model_file),
        ],
        "unstable_schema: true\n",
    );

    // Check the model file (handles[1])
    let model_sf = handles[1];
    let diags = file_diagnostics(&db, ws, model_sf);

    let advisory_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MissingProvenancePushdownAdvisory))
        .collect();
    assert!(
        !advisory_diags.is_empty(),
        "expected at least one MissingProvenancePushdownAdvisory diagnostic \
         when transparent function lacks provenance and WHERE is present, \
         got: {diags:#?}"
    );
}

/// Test 2 — function WITH provenance + WHERE → NO Hint.
#[test]
fn provenance_present_no_advisory() {
    let fn_file = "---\nprovenance: { total: [src.total] }\n---\n\
                   smelt.define my_func(src: TableExpr) -> TableExpr AS (SELECT * FROM src)";
    let model_file = "SELECT * FROM smelt.fn.my_func(smelt.ref('tbl')) WHERE total > 100";

    let (db, ws, handles) = make_db_with_files(
        &[
            ("functions/my_func_with_prov.sql", fn_file),
            ("models/model_with_prov.sql", model_file),
        ],
        "unstable_schema: true\n",
    );

    let model_sf = handles[1];
    let diags = file_diagnostics(&db, ws, model_sf);

    let advisory_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MissingProvenancePushdownAdvisory))
        .collect();
    assert!(
        advisory_diags.is_empty(),
        "expected ZERO MissingProvenancePushdownAdvisory diagnostics \
         when function has declared provenance, got: {advisory_diags:#?}"
    );
}

// ============================================================================
// Part (b): ExternFragmentParamUnsupported
// ============================================================================

/// Test 3 — `smelt.extern` with SelectItems param → rejected.
#[test]
fn extern_with_selectitems_param_rejected() {
    let content = "smelt.extern foo(items: SelectItems<Agg>) -> TableExpr";
    let diags = get_diagnostics_single(content);

    let bad_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ExternFragmentParamUnsupported))
        .collect();
    assert!(
        !bad_diags.is_empty(),
        "expected at least one ExternFragmentParamUnsupported diagnostic \
         for SelectItems param, got: {diags:#?}"
    );
}

/// Test 4 — `smelt.extern` with OrderSpec param → rejected.
#[test]
fn extern_with_orderspec_param_rejected() {
    let content = "smelt.extern foo(spec: OrderSpec) -> TableExpr";
    let diags = get_diagnostics_single(content);

    let bad_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ExternFragmentParamUnsupported))
        .collect();
    assert!(
        !bad_diags.is_empty(),
        "expected at least one ExternFragmentParamUnsupported diagnostic \
         for OrderSpec param, got: {diags:#?}"
    );
}

/// Test 5 — `smelt.extern` with Expr<Text> and TableExpr params → no rejection.
#[test]
fn extern_with_expr_or_tableexpr_param_unchanged() {
    let content = "smelt.extern my_extern_func(s: Expr<Text>, t: TableExpr) -> TableExpr";
    let diags = get_diagnostics_single(content);

    let bad_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ExternFragmentParamUnsupported))
        .collect();
    assert!(
        bad_diags.is_empty(),
        "expected ZERO ExternFragmentParamUnsupported diagnostics \
         for Expr<Text> and TableExpr params, got: {bad_diags:#?}"
    );
}
