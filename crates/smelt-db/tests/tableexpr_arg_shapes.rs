//! Phase 46 (smelt-functions) — `TableExpr` argument shape resolution.
//!
//! A function whose argument is a CTE reference, a derived table
//! `(SELECT …) AS d`, or an inline subquery `(SELECT …)` must have its
//! schema resolved by the call-site so the body's bare-column resolver
//! sees the caller-supplied columns. Closes review finding #20.
//!
//! Tests:
//!   1. `tableexpr_arg_from_cte` — CTE reference `x` (`WITH x AS …`).
//!   2. `tableexpr_arg_from_derived_table` — `(SELECT …) AS d`.
//!   3. `tableexpr_arg_inline_subquery` — `(SELECT …)` without alias.
//!   4. `tableexpr_arg_unrecognised_shape_errors_clearly` — literal `42`
//!      surfaces an `ArgTypeMismatch` at the argument span.
//!   5. `tableexpr_arg_from_smelt_ref_still_works` — Phase 15 baseline
//!      regression guard.

use std::path::PathBuf;

use smelt_db::{
    file_diagnostics, Database, DiagnosticCode, DiagnosticSeverity, SourceFile, Workspace,
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

const ADD_MARGIN_FN: &str =
    "smelt.define add_margin(t: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (\
    SELECT t.revenue, t.cost, t.revenue - t.cost AS margin FROM t\
)\n";

#[test]
fn tableexpr_arg_from_cte() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let model_path = root.join("models").join("uses_cte.sql");
    let model_src = "WITH x AS (SELECT CAST(100 AS DECIMAL(18,2)) AS revenue, CAST(30 AS DECIMAL(18,2)) AS cost) \
        SELECT margin FROM smelt.functions.add_margin(x)\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(fn_path, ADD_MARGIN_FN), (model_path, model_src)],
    );
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert!(
        bad.is_empty(),
        "expected no errors when arg is a CTE reference; got {bad:#?}"
    );
}

#[test]
fn tableexpr_arg_from_derived_table() {
    // Derived-table flavour: subquery selecting from a real upstream
    // model, exercising the FROM-clause inside the inner SELECT (vs the
    // literal-only inline subquery in test 3). The smelt parser does
    // not accept a trailing `AS d` alias on a subquery in expression
    // position, so we lean on the inner FROM clause to make the
    // "derived" character of the argument explicit.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let upstream_path = root.join("models").join("rev_cost.sql");
    let model_path = root.join("models").join("uses_derived.sql");
    let upstream_src = "SELECT \
        CAST(NULL AS DECIMAL(18, 2)) AS revenue, \
        CAST(NULL AS DECIMAL(18, 2)) AS cost\n";
    // Phase 4: use path form (smelt.ref() is removed).
    let model_src = "SELECT margin FROM smelt.functions.add_margin(\
        (SELECT revenue, cost FROM smelt.models.rev_cost)\
    )\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, ADD_MARGIN_FN),
            (upstream_path, upstream_src),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert!(
        bad.is_empty(),
        "expected no errors when arg is a derived-table subquery; got {bad:#?}"
    );
}

#[test]
fn tableexpr_arg_inline_subquery() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let model_path = root.join("models").join("uses_inline_sub.sql");
    let model_src = "SELECT margin FROM smelt.functions.add_margin(\
        (SELECT CAST(100 AS DECIMAL(18,2)) AS revenue, CAST(30 AS DECIMAL(18,2)) AS cost)\
    )\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(fn_path, ADD_MARGIN_FN), (model_path, model_src)],
    );
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert!(
        bad.is_empty(),
        "expected no errors when arg is an inline subquery; got {bad:#?}"
    );
}

#[test]
fn tableexpr_arg_unrecognised_shape_errors_clearly() {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let model_path = root.join("models").join("uses_literal.sql");
    let model_src = "SELECT * FROM smelt.functions.add_margin(42)\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[(fn_path, ADD_MARGIN_FN), (model_path, model_src)],
    );
    let model_file = files[1];

    let diags = file_diagnostics(&db, ws, model_file);
    let mismatches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ArgTypeMismatch))
        .filter(|d| d.message.contains("TableExpr"))
        .collect();
    assert_eq!(
        mismatches.len(),
        1,
        "expected exactly one ArgTypeMismatch diagnostic mentioning TableExpr; got {diags:#?}"
    );
    let m = mismatches[0];
    // The arg span anchors at `42` — verify by re-resolving the offset
    // in the model source. We accept the diagnostic anchoring at the
    // literal (start position) rather than buried inside the body.
    let arg_text_start = model_src.find("42").expect("`42` literal in source");
    assert_eq!(
        usize::from(m.range.start()),
        arg_text_start,
        "diagnostic should anchor at the `42` literal byte offset; got {:?}",
        m.range
    );
}

#[test]
fn tableexpr_arg_from_smelt_ref_still_works() {
    // Regression: Phase 15 baseline. Caller passes `smelt.models.orders`.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("uses_ref.sql");
    let orders_src = "SELECT \
        CAST(NULL AS DECIMAL(18, 2)) AS revenue, \
        CAST(NULL AS DECIMAL(18, 2)) AS cost\n";
    // Phase 4: use path form (smelt.ref() is removed).
    let model_src = "SELECT margin FROM smelt.functions.add_margin(smelt.models.orders)\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, ADD_MARGIN_FN),
            (orders_path, orders_src),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert!(
        bad.is_empty(),
        "expected no errors for smelt.ref-style arg (Phase 15 baseline); got {bad:#?}"
    );
}
