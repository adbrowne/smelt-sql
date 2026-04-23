//! Phase 15 (smelt-functions) — TableExpr parameter body-check with
//! row polymorphism + shadow warnings.
//!
//! A `TableExpr` parameter introduces the caller-supplied table's schema
//! into the body's SQL FROM scope. Bare column references resolve
//! through standard SQL column resolution when no parameter name
//! matches (§16 #7). Shadow warnings fire when a parameter name matches
//! a column in the caller-supplied schema (§16 #1).
//!
//! These integration tests verify, against a real call site:
//!   1. `add_margin_body_checks_ok` — a caller passing a `{revenue,
//!      cost}` model produces zero diagnostics.
//!   2. `bare_column_resolves_from_tableexpr_schema` — bare `revenue`
//!      inside the body resolves to the caller's schema.
//!   3. `missing_column_at_call_site` — a caller lacking `revenue`
//!      emits `UnknownIdentifier` anchored at the bare ref, carrying
//!      an `ExpansionFrames` payload rooted at the call site.
//!   4. `param_shadows_column_emits_warning` — an `Expr<Text>`
//!      parameter whose name overlaps a caller-schema column produces a
//!      `Severity::Warning` / `ParameterShadowsColumn` diagnostic at
//!      the param decl. Body still typechecks clean with the parameter
//!      as `Expr<Text>`.
//!   5. `qualified_access_escapes_shadow` — `source.user_id` still
//!      resolves to the column despite the shadow warning.

use std::path::PathBuf;

use smelt_db::{
    file_diagnostics, Database, DiagnosticCode, DiagnosticData, DiagnosticSeverity, SourceFile,
    Workspace,
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

/// A canonical `add_margin` function used in several tests below.
/// `TableExpr` (bare, no row-requirement) is a Phase-15-legal parameter
/// sort: the caller's schema is bound as a FROM-scope during body
/// checking.
const ADD_MARGIN_SRC: &str = "smelt.define add_margin(source: TableExpr) \
     -> TableExpr AS (SELECT source.*, revenue - cost AS margin FROM source)\n";

/// Build a workspace with an `orders` model (renamed from the source)
/// so `smelt.ref('orders')` can resolve to its schema without needing
/// the full source-lookup machinery.
fn orders_model_sql(columns: &[(&str, &str)]) -> String {
    // Produce a simple SELECT that casts each column to its declared
    // type — `smelt.ref('orders')` will pick up the resulting schema.
    let projections = columns
        .iter()
        .map(|(name, ty)| format!("CAST(NULL AS {ty}) AS {name}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT {projections}\n")
}

#[test]
fn add_margin_body_checks_ok() {
    // TDD test 1: caller passes a model with `{revenue, cost}` — body
    // checks clean. Bare `revenue` and `cost` in the body resolve via
    // the TableExpr parameter's caller-supplied schema.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("margin_report.sql");
    let model_src = "SELECT order_id, margin FROM smelt.fn.add_margin(smelt.ref('orders')) AS m\n";

    let orders_sql = orders_model_sql(&[
        ("order_id", "BIGINT"),
        ("revenue", "DECIMAL(18, 2)"),
        ("cost", "DECIMAL(18, 2)"),
    ]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, ADD_MARGIN_SRC),
            (orders_path, orders_sql.as_str()),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::ArgTypeMismatch)
                    | Some(DiagnosticCode::MissingArgument)
                    | Some(DiagnosticCode::UnknownSmeltFn)
                    | Some(DiagnosticCode::FunctionBodyTypeMismatch)
                    | Some(DiagnosticCode::UnknownIdentifier)
                    | Some(DiagnosticCode::ParameterShadowsColumn)
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "expected no function diagnostics on clean call, got {bad:#?}"
    );
}

#[test]
fn bare_column_resolves_from_tableexpr_schema() {
    // TDD test 2: bare `revenue` / `cost` inside `add_margin`'s body
    // must resolve to the caller's schema — i.e. zero UnknownIdentifier
    // diagnostics surface through the expansion path when a caller
    // supplies those columns.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("margin_report.sql");
    let model_src = "SELECT margin FROM smelt.fn.add_margin(smelt.ref('orders')) AS m\n";

    let orders_sql = orders_model_sql(&[
        ("order_id", "BIGINT"),
        ("revenue", "DECIMAL(18, 2)"),
        ("cost", "DECIMAL(18, 2)"),
    ]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, ADD_MARGIN_SRC),
            (orders_path, orders_sql.as_str()),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let unknowns: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownIdentifier))
        .collect();
    assert!(
        unknowns.is_empty(),
        "bare `revenue`/`cost` should resolve through the TableExpr schema, got {unknowns:#?}"
    );
}

#[test]
fn missing_column_at_call_site() {
    // TDD test 3: caller passes a table lacking `revenue`; the body's
    // bare `revenue` reference emits `UnknownIdentifier`. The emitted
    // diagnostic must carry an ExpansionFrames payload rooted at the
    // call site so the LSP can render a stack pointing back to the
    // `add_margin` frame.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("margin_report.sql");
    let model_src = "SELECT margin FROM smelt.fn.add_margin(smelt.ref('orders')) AS m\n";

    // Missing revenue — cost still present.
    let orders_sql = orders_model_sql(&[("order_id", "BIGINT"), ("cost", "DECIMAL(18, 2)")]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, ADD_MARGIN_SRC),
            (orders_path, orders_sql.as_str()),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let unknowns: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::UnknownIdentifier)
                && d.message.contains("revenue")
                && matches!(&d.data, Some(DiagnosticData::ExpansionFrames(_)))
        })
        .collect();
    assert!(
        !unknowns.is_empty(),
        "expected at least one UnknownIdentifier(revenue) with ExpansionFrames, got {diags:#?}"
    );

    // The top frame (outermost, last element) must name `add_margin`.
    let diag = unknowns[0];
    let frames = match &diag.data {
        Some(DiagnosticData::ExpansionFrames(frames)) => frames,
        _ => unreachable!(),
    };
    assert_eq!(
        frames.last().map(|f| f.function.as_str()),
        Some("add_margin"),
        "outermost frame should be add_margin, got {frames:#?}"
    );
}

#[test]
fn param_shadows_column_emits_warning() {
    // TDD test 4: `f(user_id: Expr<Text>, source: TableExpr)` called
    // with a source whose schema has `user_id` — emit a Warning /
    // ParameterShadowsColumn at the parameter's decl range (not at the
    // usage site inside the body). Body still typechecks clean because
    // `user_id` inside the body resolves to the `Expr<Text>`
    // parameter.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("f.sql");
    let fn_src = "smelt.define f(user_id: Expr<Text>, source: TableExpr) \
         AS (SELECT user_id FROM source)\n";
    let orders_path = root.join("models").join("events.sql");
    let model_path = root.join("models").join("uses_f.sql");
    let model_src = "SELECT * FROM smelt.fn.f('abc', smelt.ref('events')) AS e\n";

    let orders_sql = orders_model_sql(&[("user_id", "BIGINT"), ("event_type", "TEXT")]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, fn_src),
            (orders_path, orders_sql.as_str()),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let shadow: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ParameterShadowsColumn))
        .collect();
    assert_eq!(
        shadow.len(),
        1,
        "expected exactly one ParameterShadowsColumn warning, got {diags:#?}"
    );
    let diag = shadow[0];
    assert_eq!(
        diag.severity,
        DiagnosticSeverity::Warning,
        "ParameterShadowsColumn should be a warning, got {diag:#?}"
    );
    assert!(
        diag.message.contains("user_id"),
        "shadow warning should mention the shadowing param, got: {}",
        diag.message
    );

    // Body must still typecheck clean — no UnknownIdentifier on
    // `user_id` (it resolves to the Expr<Text> parameter first).
    let unknowns: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::UnknownIdentifier) && d.message.contains("user_id")
        })
        .collect();
    assert!(
        unknowns.is_empty(),
        "body should typecheck clean — `user_id` resolves to the parameter, got {unknowns:#?}"
    );
}

#[test]
fn qualified_access_escapes_shadow() {
    // TDD test 5: same setup as test 4, but body uses `source.user_id`
    // — qualified access resolves to the caller-schema column
    // regardless of the shadow. The shadow warning still fires (we
    // detect shadowing structurally, not based on body usage).
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("g.sql");
    let fn_src = "smelt.define g(user_id: Expr<Text>, source: TableExpr) \
         AS (SELECT source.user_id FROM source)\n";
    let orders_path = root.join("models").join("events.sql");
    let model_path = root.join("models").join("uses_g.sql");
    let model_src = "SELECT * FROM smelt.fn.g('abc', smelt.ref('events')) AS e\n";

    let orders_sql = orders_model_sql(&[("user_id", "BIGINT"), ("event_type", "TEXT")]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, fn_src),
            (orders_path, orders_sql.as_str()),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);

    // Qualified access `source.user_id` must resolve; no
    // UnknownIdentifier on it.
    let unknowns: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownIdentifier))
        .collect();
    assert!(
        unknowns.is_empty(),
        "qualified access `source.user_id` should resolve, got {unknowns:#?}"
    );
    // Shadow warning must still fire regardless of usage pattern.
    let shadow: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ParameterShadowsColumn))
        .collect();
    assert_eq!(
        shadow.len(),
        1,
        "shadow warning is structural — must still fire when body uses qualified access, got {diags:#?}"
    );
}
