//! Phase 19 — Context-binding syntax (`Expr<T, ctx>`).
//!
//! Tests that:
//!   1. `context_binding_parsed_into_signature` — `filters: Expr<Boolean, source>`
//!      stores `context_name = "source"` in `ParamSpec.context`.
//!   2. `context_name_resolves_to_tableexpr_param` — when `source` is also a
//!      `TableExpr` param in the same signature, no `UnknownContext` diagnostic
//!      fires.
//!   3. `invalid_context_name_errors_at_definition_time` — a context name that
//!      matches no parameter in the same function → `UnknownContext` diagnostic.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(
        project_root.clone(),
        "version: 1\nsources: []\n".to_string(),
    );
    let mut handles = Vec::new();
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

#[test]
fn context_binding_parsed_into_signature() {
    // Phase 19 TDD test 1: `filters: Expr<Boolean, source>` stores
    // `context_name = "source"` in ParamSpec.context.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("filter_fn.sql");
    let sql = "smelt.define filter_fn( \
       source: TableExpr, \
       filters: Expr<Boolean, source> \
     ) -> TableExpr AS (SELECT * FROM source WHERE filters)\n";

    let (db, _ws, files) = build_db(root, &[(fn_path, sql)]);
    let sigs = smelt_db::file_signature_inputs(&db, files[0]);

    assert_eq!(sigs.len(), 1, "should have one function");
    let sig = &sigs[0];

    let filters_param = sig
        .params
        .iter()
        .find(|p| p.name == "filters")
        .expect("filters param present");

    assert!(
        filters_param.context.is_some(),
        "filters param should carry context binding"
    );
    let ctx = filters_param.context.as_ref().unwrap();
    assert_eq!(
        ctx.name(),
        "source",
        "context name should be 'source', got {:?}",
        ctx.name()
    );
}

#[test]
fn context_name_resolves_to_tableexpr_param() {
    // Phase 19 TDD test 2: when `source` is a `TableExpr` param in the
    // same function, no `UnknownContext` diagnostic fires.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("filter_fn.sql");
    let sql = "smelt.define filter_fn( \
       source: TableExpr, \
       filters: Expr<Boolean, source> \
     ) -> TableExpr AS (SELECT * FROM source WHERE filters)\n";

    let (db, ws, files) = build_db(root, &[(fn_path, sql)]);
    let diags = file_diagnostics(&db, ws, files[0]);
    let unknown_ctx: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownContext))
        .collect();
    assert!(
        unknown_ctx.is_empty(),
        "should not emit UnknownContext when context names a param; got {unknown_ctx:#?}"
    );
}

#[test]
fn invalid_context_name_errors_at_definition_time() {
    // Phase 19 TDD test 3 (originally test 4): a context name that matches
    // no parameter in the same function → `UnknownContext` diagnostic.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("bad_ctx.sql");
    let sql = "smelt.define bad_ctx( \
       source: TableExpr, \
       filters: Expr<Boolean, nonexistent> \
     ) -> TableExpr AS (SELECT * FROM source WHERE filters)\n";

    let (db, ws, files) = build_db(root, &[(fn_path, sql)]);
    let diags = file_diagnostics(&db, ws, files[0]);
    let unknown_ctx: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownContext))
        .collect();
    assert_eq!(
        unknown_ctx.len(),
        1,
        "should emit one UnknownContext for 'nonexistent'; got {unknown_ctx:#?}"
    );
    assert!(
        unknown_ctx[0].message.contains("nonexistent"),
        "error message should mention the bad context name; got {:?}",
        unknown_ctx[0].message
    );
}
