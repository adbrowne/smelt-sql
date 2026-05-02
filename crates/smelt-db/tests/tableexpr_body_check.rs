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
//!
//! Phase 16 extends this file with row-requirement annotation tests:
//! `TableExpr<{col: Type, ..r}>` is verified at the call site *before*
//! body expansion. See the `row_requirement_*` tests at the bottom of
//! this file.
//!
//! Phase 17 extends this file with the `sessionize` end-to-end suite.
//! A `TableExpr`-returning function that uses `LAG()` and `SUM() OVER
//! (…)` in its body must:
//!
//!   * body-check clean against a caller that supplies matching columns;
//!   * synthesise an output schema that unions `source.*` with the
//!     body's explicit projections;
//!   * accept `WindowExpr`-kinded expressions in SELECT position;
//!   * apply parameter defaults when a caller omits the argument.
//!
//! See the `sessionize_*` tests at the bottom of this file.

use std::path::PathBuf;

use smelt_db::{
    file_diagnostics, infer_tableexpr_return_schema, model_schema, parse_file, typed_model_schema,
    Database, DiagnosticCode, DiagnosticData, DiagnosticSeverity, SourceFile, Workspace,
};
use smelt_parser::ast::{File as AstFile, SmeltDefine};
use smelt_types::DataType;

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
/// so `smelt.models.orders` can resolve to its schema without needing
/// the full source-lookup machinery.
fn orders_model_sql(columns: &[(&str, &str)]) -> String {
    // Produce a simple SELECT that casts each column to its declared
    // type — `smelt.models.orders` will pick up the resulting schema.
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
    let model_src =
        "SELECT order_id, margin FROM smelt.functions.add_margin(smelt.models.orders) AS m\n";

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
    let model_src = "SELECT margin FROM smelt.functions.add_margin(smelt.models.orders) AS m\n";

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
    let model_src = "SELECT margin FROM smelt.functions.add_margin(smelt.models.orders) AS m\n";

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
    let model_src = "SELECT * FROM smelt.functions.f('abc', smelt.models.events) AS e\n";

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
    let model_src = "SELECT * FROM smelt.functions.g('abc', smelt.models.events) AS e\n";

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

// =====================================================================
// Phase 16 — Row-requirement annotations: TableExpr<{…}> pre-expansion
// =====================================================================

/// A row-requirement-annotated `add_margin` used by several Phase-16
/// tests. Declares `source: TableExpr<{revenue: Numeric, cost: Numeric}>`.
const ADD_MARGIN_REQ_SRC: &str =
    "smelt.define add_margin(source: TableExpr<{revenue: Numeric, cost: Numeric}>) \
     -> TableExpr AS (SELECT source.*, revenue - cost AS margin FROM source)\n";

#[test]
fn row_requirement_satisfied_by_superset_schema() {
    // Phase 16 TDD test 1: caller supplies `{order_id, revenue, cost,
    // extra_col}` — a strict superset of `{revenue: Numeric, cost:
    // Numeric}`. Zero diagnostics; call-site row-requirement check
    // passes and the body check runs normally.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("margin_report.sql");
    let model_src = "SELECT * FROM smelt.functions.add_margin(smelt.models.orders) AS m\n";

    let orders_sql = orders_model_sql(&[
        ("order_id", "BIGINT"),
        ("revenue", "DECIMAL(18, 2)"),
        ("cost", "DECIMAL(18, 2)"),
        ("extra_col", "TEXT"),
    ]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, ADD_MARGIN_REQ_SRC),
            (orders_path, orders_sql.as_str()),
            (model_path, model_src),
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
                    | Some(DiagnosticCode::RowRequirementUnsatisfied)
                    | Some(DiagnosticCode::InvalidFunctionTypeRef)
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "superset-schema caller should satisfy row requirement; got {bad:#?}"
    );
}

#[test]
fn row_requirement_missing_column_errors_at_call_site() {
    // Phase 16 TDD test 2: caller supplies `{order_id, revenue}` only
    // — missing `cost`. A `RowRequirementUnsatisfied` diagnostic must
    // fire naming `cost`. The body must NOT be re-checked, so no
    // `UnknownIdentifier` for `revenue - cost` in the body. Frame
    // stack rooted at call site.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("margin_report.sql");
    let model_src = "SELECT * FROM smelt.functions.add_margin(smelt.models.orders) AS m\n";

    // Missing `cost` — only revenue present.
    let orders_sql = orders_model_sql(&[("order_id", "BIGINT"), ("revenue", "DECIMAL(18, 2)")]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, ADD_MARGIN_REQ_SRC),
            (orders_path, orders_sql.as_str()),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);

    // 1. Exactly one RowRequirementUnsatisfied mentioning `cost`.
    let req_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::RowRequirementUnsatisfied) && d.message.contains("cost")
        })
        .collect();
    assert!(
        !req_diags.is_empty(),
        "expected RowRequirementUnsatisfied mentioning `cost`, got {diags:#?}"
    );

    // 2. Body was NOT re-checked — no UnknownIdentifier on `cost` from
    //    the body's bare-reference expansion.
    let body_unknowns: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::UnknownIdentifier)
                && matches!(&d.data, Some(DiagnosticData::ExpansionFrames(_)))
        })
        .collect();
    assert!(
        body_unknowns.is_empty(),
        "row requirement failure must short-circuit body check; got cascade UnknownIdentifier diagnostics {body_unknowns:#?}"
    );
}

#[test]
fn row_requirement_wrong_type_errors_at_call_site() {
    // Phase 16 TDD test 3: caller supplies `{revenue: Text, cost:
    // Decimal}`. `Text` does not satisfy the `Numeric` constraint on
    // `revenue`. Expect a RowRequirementUnsatisfied diagnostic naming
    // `revenue` and the expected constraint `Numeric`.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("margin_report.sql");
    let model_src = "SELECT * FROM smelt.functions.add_margin(smelt.models.orders) AS m\n";

    // `revenue` is Text — violates the Numeric constraint.
    let orders_sql = orders_model_sql(&[
        ("order_id", "BIGINT"),
        ("revenue", "TEXT"),
        ("cost", "DECIMAL(18, 2)"),
    ]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, ADD_MARGIN_REQ_SRC),
            (orders_path, orders_sql.as_str()),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let req_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::RowRequirementUnsatisfied)
                && d.message.contains("revenue")
                && d.message.contains("Numeric")
        })
        .collect();
    assert!(
        !req_diags.is_empty(),
        "expected RowRequirementUnsatisfied mentioning `revenue` and `Numeric`, got {diags:#?}"
    );
}

#[test]
fn row_tail_allows_extra_columns() {
    // Phase 16 TDD test 4: parameter declared
    // `TableExpr<{revenue: Numeric, ..r}>`. The caller supplies
    // `{revenue, cost, notes}` — extras allowed by the row-tail. No
    // diagnostics. (Observability of `r` via an accessor is exercised
    // by the unit test in `smelt-types`; here we assert zero
    // diagnostics end-to-end.)
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_revenue.sql");
    let fn_src = "smelt.define add_revenue(source: TableExpr<{revenue: Numeric, ..r}>) \
         -> TableExpr AS (SELECT source.*, revenue AS r2 FROM source)\n";
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("tail_report.sql");
    let model_src = "SELECT * FROM smelt.functions.add_revenue(smelt.models.orders) AS m\n";

    let orders_sql = orders_model_sql(&[
        ("revenue", "DECIMAL(18, 2)"),
        ("cost", "DECIMAL(18, 2)"),
        ("notes", "TEXT"),
    ]);

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
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::RowRequirementUnsatisfied)
                    | Some(DiagnosticCode::UnknownIdentifier)
                    | Some(DiagnosticCode::ArgTypeMismatch)
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "named-tail `..r` should accept extras; got {bad:#?}"
    );
}

#[test]
fn row_tail_anonymous_allows_any_extras() {
    // Phase 16 TDD test 5: parameter declared
    // `TableExpr<{revenue: Numeric, ..}>`. Anonymous tail accepts the
    // same extras as a named tail but records no binding. Zero
    // diagnostics.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("anon_tail.sql");
    let fn_src = "smelt.define anon_tail(source: TableExpr<{revenue: Numeric, ..}>) \
         -> TableExpr AS (SELECT source.* FROM source)\n";
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("tail_report.sql");
    let model_src = "SELECT * FROM smelt.functions.anon_tail(smelt.models.orders) AS m\n";

    let orders_sql = orders_model_sql(&[
        ("revenue", "DECIMAL(18, 2)"),
        ("cost", "DECIMAL(18, 2)"),
        ("notes", "TEXT"),
    ]);

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
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::RowRequirementUnsatisfied)
                    | Some(DiagnosticCode::UnknownIdentifier)
                    | Some(DiagnosticCode::ArgTypeMismatch)
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "anonymous tail `..` should accept extras; got {bad:#?}"
    );
}

#[test]
fn row_requirement_at_tier1_function_body_still_checks_on_expansion() {
    // Phase 16 TDD test 6: when the row requirement is satisfied the
    // body is still checked. Prove this by using a body that references
    // a non-existent bare column (`missing_col`) — the body check must
    // still fire `UnknownIdentifier` because we didn't short-circuit.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("with_body_bug.sql");
    let fn_src =
        "smelt.define with_body_bug(source: TableExpr<{revenue: Numeric, cost: Numeric}>) \
         -> TableExpr AS (SELECT source.*, missing_col AS x FROM source)\n";
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("report.sql");
    let model_src = "SELECT * FROM smelt.functions.with_body_bug(smelt.models.orders) AS m\n";

    let orders_sql = orders_model_sql(&[("revenue", "DECIMAL(18, 2)"), ("cost", "DECIMAL(18, 2)")]);

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

    // Row-requirement is satisfied, so we expect NO
    // RowRequirementUnsatisfied diagnostic.
    let req_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::RowRequirementUnsatisfied))
        .collect();
    assert!(
        req_diags.is_empty(),
        "row requirement is satisfied; should not emit RowRequirementUnsatisfied, got {req_diags:#?}"
    );

    // Body check must still run — `missing_col` surfaces as a cascade
    // UnknownIdentifier with ExpansionFrames.
    let body_unknowns: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::UnknownIdentifier) && d.message.contains("missing_col")
        })
        .collect();
    assert!(
        !body_unknowns.is_empty(),
        "body check must still run when row requirement is satisfied; got {diags:#?}"
    );
}

// =====================================================================
// Phase 17 — sessionize: TableExpr output-schema inference + defaults
// =====================================================================

/// Canonical `sessionize` used in several Phase-17 tests. The body uses
/// both `LAG()` and `SUM() OVER (…)` — two window-kind expressions that
/// must be accepted in SELECT-list position. The `gap` parameter has a
/// default value so tests can exercise default-value expansion.
const SESSIONIZE_SRC: &str = "smelt.define sessionize(\
    source: TableExpr, \
    user_col: Expr<Text>, \
    ts_col: Expr<Timestamp>, \
    gap: Expr<Interval> = INTERVAL '30 minutes'\
) -> TableExpr AS (\
    SELECT source.*, \
        SUM(CASE WHEN ts_col - LAG(ts_col) OVER (PARTITION BY user_col ORDER BY ts_col) > gap THEN 1 ELSE 0 END) \
        OVER (PARTITION BY user_col ORDER BY ts_col) AS session_id \
    FROM source\
)\n";

/// Build an `events` model exposing `{user_id: Text, event_time:
/// Timestamp}` so `smelt.models.events` resolves to a schema compatible
/// with `sessionize`'s inputs.
fn events_model_sql() -> String {
    orders_model_sql(&[
        ("user_id", "TEXT"),
        ("event_time", "TIMESTAMP"),
        ("event_type", "TEXT"),
    ])
}

#[test]
fn sessionize_body_types_clean() {
    // Phase 17 TDD test 1: define `sessionize(source, user_col, ts_col,
    // gap=…)` with a body that mixes `LAG()` and `SUM() OVER (…)`. When
    // the caller supplies `{user_id, event_time, …}`, the body check
    // must produce zero diagnostics.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("sessionize.sql");
    let events_path = root.join("models").join("events.sql");
    let model_path = root.join("models").join("sessions_report.sql");
    let model_src = "SELECT * FROM smelt.functions.sessionize(\
         smelt.models.events, \
         user_col => user_id, \
         ts_col => event_time\
     ) AS s\n";

    let events_sql = events_model_sql();
    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, SESSIONIZE_SRC),
            (events_path, events_sql.as_str()),
            (model_path, model_src),
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
                    | Some(DiagnosticCode::WindowInScalarContext)
                    | Some(DiagnosticCode::RowRequirementUnsatisfied)
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "expected clean body-check on sessionize call; got {bad:#?}"
    );
}

#[test]
fn sessionize_output_schema_inferred() {
    // Phase 17 TDD test 2: pure-function-level assertion that the
    // `infer_tableexpr_return_schema` helper walks the body's final
    // SELECT and produces `source.*` + `session_id: BigInt`. We build a
    // minimal context with the caller-bound schema and expect the
    // returned `ModelSchema` to carry that union.
    use smelt_db::TypeContext;
    use smelt_parser::{parse, strip_frontmatter};
    use smelt_types::TypedColumn;

    let clean = strip_frontmatter(SESSIONIZE_SRC).to_string();
    let p = parse(&clean);
    let ast = AstFile::cast(p.syntax()).expect("FILE");
    let define: SmeltDefine = ast.defines().next().expect("one SmeltDefine");
    let body = define.body().expect("body present");
    let select_stmt = body.select_stmt().expect("body is SELECT");

    // Seed a ctx with the caller schema on `source` and the Expr<T>
    // params on `user_col`, `ts_col`, `gap`.
    let mut ctx = TypeContext::new();
    let caller_cols: Vec<(String, TypedColumn)> = vec![
        ("user_id".to_string(), TypedColumn::nullable(DataType::Text)),
        (
            "event_time".to_string(),
            TypedColumn::nullable(DataType::Timestamp {
                with_timezone: false,
            }),
        ),
    ];
    ctx.add_tableexpr_param("source", &caller_cols);
    ctx.add_function_param("user_col", TypedColumn::nullable(DataType::Text));
    ctx.add_function_param(
        "ts_col",
        TypedColumn::nullable(DataType::Timestamp {
            with_timezone: false,
        }),
    );
    ctx.add_function_param("gap", TypedColumn::nullable(DataType::Interval));

    let schema = infer_tableexpr_return_schema(&select_stmt, &ctx)
        .expect("infer_tableexpr_return_schema must produce Some");

    let col_names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
    // `source.*` expands to the caller's columns.
    assert!(
        col_names.contains(&"user_id"),
        "output schema must carry user_id; got {col_names:?}"
    );
    assert!(
        col_names.contains(&"event_time"),
        "output schema must carry event_time; got {col_names:?}"
    );
    // Explicit alias.
    assert!(
        col_names.contains(&"session_id"),
        "output schema must carry session_id; got {col_names:?}"
    );
    // The inferred type of `session_id` must be BigInt (SUM over
    // Integer → BigInt per the built-in aggregate-widening rule).
    let session_id = schema
        .columns
        .iter()
        .find(|c| c.name == "session_id")
        .expect("session_id present");
    let dt = session_id
        .data_type
        .as_ref()
        .map(|tc| tc.data_type.clone())
        .unwrap_or(DataType::Unknown);
    assert!(
        matches!(dt, DataType::BigInt),
        "session_id should widen to BigInt; got {dt:?}"
    );
}

#[test]
fn sessionize_windowexpr_in_body_accepted() {
    // Phase 17 TDD test 3: body synthesising window-kind expressions
    // in SELECT position must not trigger WindowInScalarContext —
    // Phase 14's check only fires in WHERE / GROUP BY.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("sessionize.sql");
    let events_path = root.join("models").join("events.sql");
    let model_path = root.join("models").join("sessions_report.sql");
    let model_src = "SELECT * FROM smelt.functions.sessionize(\
         smelt.models.events, \
         user_col => user_id, \
         ts_col => event_time\
     ) AS s\n";

    let events_sql = events_model_sql();
    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, SESSIONIZE_SRC),
            (events_path, events_sql.as_str()),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let window_errs: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::WindowInScalarContext))
        .collect();
    assert!(
        window_errs.is_empty(),
        "window expressions in SELECT list must not trigger WindowInScalarContext; got {window_errs:#?}"
    );
}

#[test]
fn sessionize_missing_ts_col_on_source_errors() {
    // Phase 17 TDD test 4: when the caller's source schema lacks a
    // column that the body references via the `source.col` qualified
    // access pattern, the body-check surfaces the failure as a cascade
    // `UnknownIdentifier` rooted at the call site.
    //
    // We exercise this by defining a variant of `sessionize` whose
    // body references `source.ts` (a column the caller may or may not
    // supply). When the source lacks `ts`, the body's reference
    // surfaces through the expansion path — the canonical §16 #7
    // behaviour extended to the SELECT-body walker.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("sessionize_v.sql");
    // A simplified variant that uses `source.ts` directly in the body
    // so missing `ts` from the caller surfaces as UnknownIdentifier.
    let fn_src = "smelt.define sessionize_v(source: TableExpr) \
         -> TableExpr AS (\
            SELECT source.*, source.ts AS ts2 FROM source\
         )\n";
    let events_path = root.join("models").join("events.sql");
    let model_path = root.join("models").join("sessions_report.sql");
    let model_src = "SELECT * FROM smelt.functions.sessionize_v(smelt.models.events) AS s\n";

    // Missing `ts` — only user_id present.
    let events_sql = orders_model_sql(&[("user_id", "TEXT")]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, fn_src),
            (events_path, events_sql.as_str()),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let unknowns: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownIdentifier) && d.message.contains("ts"))
        .collect();
    assert!(
        !unknowns.is_empty(),
        "missing `ts` on source should surface as UnknownIdentifier with expansion frames; got {diags:#?}"
    );
    // The top frame must name `sessionize_v` so the user's editor can
    // render a Phase-12 stack trailer.
    let has_frame = unknowns.iter().any(|d| {
        matches!(
            &d.data,
            Some(DiagnosticData::ExpansionFrames(frames)) if frames.iter().any(|f| f.function == "sessionize_v")
        )
    });
    assert!(
        has_frame,
        "body cascade on missing column must carry ExpansionFrames rooted at `sessionize_v`; got {unknowns:#?}"
    );
}

#[test]
fn sessionize_default_gap_applied_when_omitted() {
    // Phase 17 TDD test 5: the caller omits `gap`; the default value
    // (an INTERVAL literal) is plumbed through, so zero diagnostics
    // surface. Previously Phase 16 emitted a MissingArgument here
    // because the default expansion wasn't wired end-to-end.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("sessionize.sql");
    let events_path = root.join("models").join("events.sql");
    let model_path = root.join("models").join("sessions_report.sql");
    let model_src = "SELECT * FROM smelt.functions.sessionize(\
         smelt.models.events, \
         user_col => user_id, \
         ts_col => event_time\
     ) AS s\n";

    let events_sql = events_model_sql();
    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, SESSIONIZE_SRC),
            (events_path, events_sql.as_str()),
            (model_path, model_src),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let missing_gap: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MissingArgument) && d.message.contains("gap"))
        .collect();
    assert!(
        missing_gap.is_empty(),
        "`gap`'s default value must be applied when omitted; got {missing_gap:#?}"
    );
    // And the overall body-level diagnostics must stay clean.
    let cascade: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(&d.data, Some(DiagnosticData::ExpansionFrames(_)))
                && matches!(
                    d.code,
                    Some(DiagnosticCode::UnknownIdentifier)
                        | Some(DiagnosticCode::FunctionBodyTypeMismatch)
                )
        })
        .collect();
    assert!(
        cascade.is_empty(),
        "default-value expansion must not introduce body cascades; got {cascade:#?}"
    );
}

#[test]
fn margin_report_explicit_columns_after_return_schema_inference() {
    // Phase 17 TDD test 6: the Phase-15 deferred item — once
    // TableExpr return-schema inference lands, callers can project
    // explicit columns from a `TableExpr`-returning call site. Assert
    // the caller model's inferred schema carries `order_id` and
    // `margin` (from the `add_margin` body's projections).
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("add_margin.sql");
    let orders_path = root.join("models").join("orders.sql");
    let model_path = root.join("models").join("margin_report.sql");
    let model_src =
        "SELECT order_id, margin FROM smelt.functions.add_margin(smelt.models.orders) AS m\n";

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

    // No `UnknownIdentifier` / `UndeclaredColumn` on `order_id` /
    // `margin` — they resolve against the inferred `add_margin`
    // output schema exposed as FROM-scope columns of the alias `m`.
    let diags = file_diagnostics(&db, ws, model_file);
    let unknowns: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::UnknownIdentifier) | Some(DiagnosticCode::UndeclaredColumn)
            ) && (d.message.contains("order_id") || d.message.contains("margin"))
        })
        .collect();
    assert!(
        unknowns.is_empty(),
        "explicit columns from a TableExpr call must resolve via the inferred return schema; got {unknowns:#?}"
    );

    // Parsing / model_schema sanity check — ensure the caller model's
    // schema carries `order_id` and `margin` columns.
    let p = parse_file(&db, model_file);
    let _ = p; // just exercising the path
    let schema = typed_model_schema(&db, ws, model_file);
    let names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"order_id"),
        "caller schema must carry order_id; got {names:?}"
    );
    assert!(
        names.contains(&"margin"),
        "caller schema must carry margin; got {names:?}"
    );

    // Also verify the bare model_schema plumbing still sees them (a
    // simple check of pre-typing schema extraction).
    let base = model_schema(&db, model_file);
    assert!(
        !base.columns.is_empty(),
        "base model schema should not be empty"
    );
}

// =====================================================================
// Phase 18 — Nested smelt.functions.* call chain: add_margin → sessionize
// =====================================================================

#[test]
fn nested_smelt_fn_call_session_id_in_scope() {
    // Phase 18 TDD test: chaining add_margin → sessionize must make
    // `session_id` visible in the outer model's FROM scope.
    //
    // This exercises `resolve_smelt_fn_call_schema` recursively:
    // outer sessionize → resolves inner add_margin → resolves orders →
    // feeds schema upward.
    let root = PathBuf::from("/fake/project");
    let add_margin_path = root.join("functions").join("add_margin.sql");
    let sessionize_path = root.join("functions").join("sessionize.sql");
    let orders_path = root.join("models").join("orders.sql");
    let pipeline_path = root.join("models").join("margin_by_session.sql");

    let orders_sql =
        "SELECT CAST(NULL AS DECIMAL(18,2)) AS revenue, CAST(NULL AS DECIMAL(18,2)) AS cost\n";
    let pipeline_sql = "SELECT session_id FROM smelt.functions.sessionize( \
       smelt.functions.add_margin(smelt.models.orders), \
       user_col => CAST('' AS VARCHAR), \
       ts_col => CAST('2020-01-01' AS TIMESTAMP) \
     ) AS s\n";

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (add_margin_path, ADD_MARGIN_REQ_SRC),
            (sessionize_path, SESSIONIZE_SRC),
            (orders_path, orders_sql),
            (pipeline_path, pipeline_sql),
        ],
    );
    let pipeline_file = files[3];

    // check_type_diagnostics accumulates UndeclaredColumn errors.
    let type_diags = smelt_db::check_type_diagnostics::accumulated::<smelt_db::DiagnosticAcc>(
        &db,
        ws,
        pipeline_file,
    );
    let errors: Vec<_> = type_diags
        .iter()
        .filter(|d| d.0.severity == DiagnosticSeverity::Error)
        .map(|d| d.0.message.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "margin_by_session pipeline should be error-free; got: {errors:#?}"
    );
}
