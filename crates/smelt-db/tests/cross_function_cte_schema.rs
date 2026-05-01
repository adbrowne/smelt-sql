//! Phase 47 — cross-function CTE schema inference.
//!
//! When a CTE body has the shape `SELECT * FROM smelt.fn.<name>(<args>)`,
//! the CTE's schema should be inferred from the callee's return schema
//! (rather than being marked opaque). Closes review finding #22.
//!
//! Tests:
//!   1. `cte_schema_inferred_from_smelt_fn_call` — wildcard-from-smelt-fn
//!      shape resolves to the callee's column list; outer SELECT
//!      against named CTE columns is clean.
//!   2. `cte_schema_typo_inside_caller_caught` — same shape, but the
//!      outer SELECT references a column not in the inferred schema;
//!      `UndeclaredColumn` fires.
//!   3. `cte_schema_inference_handles_chained_smelt_fn_calls` — a CTE
//!      body wrapping `smelt.fn.add_margin(smelt.fn.add_one(...))`
//!      resolves recursively through both calls.
//!   4. `mark_cte_opaque_still_used_as_fallback` — when the inner
//!      smelt.fn.* doesn't resolve, the opaque-CTE fallback still
//!      keeps outer column references quiet (only the
//!      `UnknownSmeltFn` error fires).

use std::path::PathBuf;

use smelt_db::{
    check_type_diagnostics, file_diagnostics, Database, DiagnosticAcc, DiagnosticCode,
    DiagnosticSeverity, SourceFile, Workspace,
};

fn build_db(
    project_root: PathBuf,
    sources_yaml: &str,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), sources_yaml.to_string());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

fn model_with_columns(columns: &[(&str, &str)]) -> String {
    let projections = columns
        .iter()
        .map(|(name, ty)| format!("CAST(NULL AS {ty}) AS {name}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT {projections}\n")
}

/// Combined diagnostics from both the file-diagnostics accumulator and
/// the type-diagnostics accumulator. Mirrors the pattern in
/// `join_alias_visibility.rs`.
fn all_diagnostics(db: &Database, ws: Workspace, file: SourceFile) -> Vec<smelt_db::Diagnostic> {
    let file_diags = file_diagnostics(db, ws, file);
    let type_diags: Vec<_> = check_type_diagnostics::accumulated::<DiagnosticAcc>(db, ws, file)
        .into_iter()
        .map(|d| d.0.clone())
        .collect();
    file_diags.into_iter().chain(type_diags).collect()
}

#[test]
fn cte_schema_inferred_from_smelt_fn_call() {
    // The CTE body has shape `SELECT * FROM smelt.fn.add_one(smelt.models.events)`.
    // The inferred schema for `x` is `add_one`'s return schema. The outer
    // SELECT projects `id` from `x` — must resolve cleanly.
    let root = PathBuf::from("/fake/project");
    let events_path = root.join("models").join("events.sql");
    let fn_path = root.join("functions").join("add_one.sql");
    let model_path = root.join("models").join("uses_add_one_cte.sql");

    let events_sql = model_with_columns(&[("id", "BIGINT")]);
    let fn_src = "smelt.define add_one(t: TableExpr) -> TableExpr AS (\
        SELECT t.id FROM t\
    )\n";
    // Phase 4: use path form (smelt.ref() is removed).
    let model_src = "WITH x AS (\
        SELECT * FROM smelt.fn.add_one(smelt.models.events)\
    ) SELECT id FROM x\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (events_path, events_sql.as_str()),
            (fn_path, fn_src),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[2];

    let diags = all_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert!(
        bad.is_empty(),
        "expected no errors after Phase 47; got {bad:#?}"
    );
}

#[test]
fn cte_schema_typo_inside_caller_caught() {
    // Same shape, but the outer SELECT references `does_not_exist` which
    // isn't on the inferred schema. Phase 47 makes this fire as
    // `UndeclaredColumn` (today it's silently swallowed by
    // `mark_cte_opaque`).
    let root = PathBuf::from("/fake/project");
    let events_path = root.join("models").join("events.sql");
    let fn_path = root.join("functions").join("add_one.sql");
    let model_path = root.join("models").join("uses_add_one_typo.sql");

    let events_sql = model_with_columns(&[("id", "BIGINT")]);
    let fn_src = "smelt.define add_one(t: TableExpr) -> TableExpr AS (\
        SELECT t.id FROM t\
    )\n";
    // Phase 4: use path form (smelt.ref() is removed).
    let model_src = "WITH x AS (\
        SELECT * FROM smelt.fn.add_one(smelt.models.events)\
    ) SELECT does_not_exist FROM x\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (events_path, events_sql.as_str()),
            (fn_path, fn_src),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[2];

    let diags = all_diagnostics(&db, ws, model_file);
    let undeclared: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::UnknownIdentifier)
                || d.code == Some(DiagnosticCode::UndeclaredColumn)
        })
        .filter(|d| d.message.contains("does_not_exist"))
        .collect();
    assert_eq!(
        undeclared.len(),
        1,
        "expected exactly one undeclared-column / unknown-identifier on `does_not_exist`; got {diags:#?}"
    );
}

#[test]
fn cte_schema_inference_handles_chained_smelt_fn_calls() {
    // CTE body wraps two smelt.fn calls: add_margin(add_one(...)).
    // Recursive resolution must walk both calls so the outer
    // SELECT can resolve `margin` (added by add_margin).
    let root = PathBuf::from("/fake/project");
    let orders_path = root.join("models").join("orders.sql");
    let inc_path = root.join("functions").join("add_one.sql");
    let margin_path = root.join("functions").join("add_margin.sql");
    let model_path = root.join("models").join("uses_chain.sql");

    let orders_sql = model_with_columns(&[
        ("order_id", "BIGINT"),
        ("revenue", "DECIMAL(18,2)"),
        ("cost", "DECIMAL(18,2)"),
    ]);
    let inc_src = "smelt.define add_one(t: TableExpr) -> TableExpr AS (\
        SELECT t.* FROM t\
    )\n";
    let margin_src = "smelt.define add_margin(src: TableExpr) -> TableExpr AS (\
        SELECT src.*, src.revenue - src.cost AS margin FROM src\
    )\n";
    // Phase 4: use path form (smelt.ref() is removed).
    let model_src = "WITH x AS (\
        SELECT * FROM smelt.fn.add_margin(smelt.fn.add_one(smelt.models.orders))\
    ) SELECT margin FROM x\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (orders_path, orders_sql.as_str()),
            (inc_path, inc_src),
            (margin_path, margin_src),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[3];

    let diags = all_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert!(
        bad.is_empty(),
        "expected no errors for chained smelt.fn calls; got {bad:#?}"
    );
}

#[test]
fn mark_cte_opaque_still_used_as_fallback() {
    // The CTE references a function that doesn't exist. The lookup
    // closure returns None; the existing opaque-CTE fallback keeps
    // outer column refs quiet so we get exactly one error
    // (the UnknownSmeltFn).
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("uses_unknown_fn.sql");

    let model_src = "WITH x AS (\
        SELECT * FROM smelt.fn.does_not_exist()\
    ) SELECT some_col FROM x\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(model_path.clone(), model_src)],
    );
    let model_file = files[0];

    let diags = all_diagnostics(&db, ws, model_file);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    let unknown_fn: Vec<_> = errors
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownSmeltFn))
        .collect();
    assert_eq!(
        unknown_fn.len(),
        1,
        "expected exactly one UnknownSmeltFn error; got {errors:#?}"
    );
    let unknown_ident: Vec<_> = errors
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::UnknownIdentifier)
                || d.code == Some(DiagnosticCode::UndeclaredColumn)
        })
        .collect();
    assert!(
        unknown_ident.is_empty(),
        "outer SELECT references against opaque CTE should not cascade UnknownIdentifier; got {unknown_ident:#?}"
    );
}
