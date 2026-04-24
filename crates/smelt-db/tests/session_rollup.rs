//! Phase 22 — `session_rollup` end-to-end (Step 4 complete).
//!
//! Tests:
//!   1. `session_rollup_body_types_clean` — `file_diagnostics` on the full
//!      `session_rollup.sql` fixture produces zero diagnostics.  Requires that
//!      `unknown_context_diagnostics_for_file` accepts CTE names as valid
//!      context references (not just parameter names).
//!   2. `caller_metrics_fragment_sees_session_id` — build a TypeContext with
//!      a `sessionized` CTE seeded with `{user_col: Text, session_id: BigInt}`,
//!      call `check_fragment_context_bindings` with `metrics => SUM(session_id)`,
//!      expect no `FragmentColumnMissing`.
//!   3. `caller_metrics_fragment_rejects_non_sessionized_column` — same setup,
//!      call with `metrics => SUM(xyz)` where `xyz` is not in `sessionized`,
//!      expect `FragmentColumnMissing`.
//!   4. `empty_default_metrics_splice_comma_elision` — a function with
//!      `metrics: SelectItems<Agg, sessionized> = ()` default and a proper
//!      CTE body produces zero diagnostics from `file_diagnostics`.

use std::collections::HashMap;
use std::path::PathBuf;

use rowan::TextRange;
use smelt_db::{
    check_fragment_context_bindings, file_diagnostics, Database, DiagnosticCode, SourceFile,
    TypeContext, TypedColumn, Workspace,
};
use smelt_parser::ast::{Expr, SelectStmt};
use smelt_parser::File as AstFile;
use smelt_types::signatures::extract_function_signatures;
use smelt_types::DataType;

// ─── shared helpers ──────────────────────────────────────────────────────────

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

fn parse_define_select(text: &str) -> (smelt_types::signatures::FunctionSig, SelectStmt, String) {
    let clean = smelt_parser::strip_frontmatter(text).to_string();
    let parse = smelt_parser::parse(&clean);
    let ast = AstFile::cast(parse.syntax()).expect("valid AstFile");
    let sig = extract_function_signatures(&ast, &clean)
        .into_iter()
        .next()
        .expect("at least one define");
    let define = ast.defines().next().expect("one SmeltDefine");
    let select = define
        .body()
        .expect("body")
        .select_stmt()
        .expect("SELECT body");
    (sig, select, clean)
}

/// Parse a standalone expression from a dummy define body.
/// Returns `(Expr, TextRange, full_dummy_text)`.
fn parse_arg_expr(expr_text: &str) -> (Expr, TextRange, String) {
    let full = format!("smelt.define _dummy() AS ({expr_text})\n");
    let clean = smelt_parser::strip_frontmatter(&full).to_string();
    let parse = smelt_parser::parse(&clean);
    let ast = AstFile::cast(parse.syntax()).expect("valid AstFile");
    let define = ast.defines().next().expect("one define");
    let body_expr = define
        .body()
        .expect("body")
        .expression()
        .expect("expression body");
    let range = body_expr.text_range();
    (body_expr, range, clean)
}

// ─── Test 1 ──────────────────────────────────────────────────────────────────

#[test]
fn session_rollup_body_types_clean() {
    // Full session_rollup definition: `metrics: SelectItems<Agg, sessionized>`
    // references the CTE name `sessionized` (not a parameter name).
    // unknown_context_diagnostics_for_file must accept CTE names.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("session_rollup.sql");
    let sql = "\
smelt.define session_rollup(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    metrics: SelectItems<Agg, sessionized> = (),
    filters: Expr<Boolean> = TRUE
) -> TableExpr AS (
    WITH sessionized AS (
        SELECT * FROM smelt.fn.sessionize(source, user_col, ts_col, gap)
    )
    SELECT
        user_col, session_id,
        MIN(ts_col) AS session_start, MAX(ts_col) AS session_end,
        COUNT(*) AS event_count,
        metrics
    FROM sessionized
    WHERE filters
    GROUP BY user_col, session_id
)
";
    let (db, ws, files) = build_db(root, &[(fn_path, sql)]);
    let diags = file_diagnostics(&db, ws, files[0]);

    let unknown_ctx_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownContext))
        .collect();
    assert!(
        unknown_ctx_diags.is_empty(),
        "CTE name `sessionized` should be accepted as valid context; got {unknown_ctx_diags:#?}"
    );
}

// ─── Test 2 ──────────────────────────────────────────────────────────────────

#[test]
fn caller_metrics_fragment_sees_session_id() {
    // Build a body-ctx with `sessionized` as a CTE containing
    // {user_col: Text, session_id: BigInt}. Then call
    // check_fragment_context_bindings with metrics => SUM(session_id).
    // Expect no FragmentColumnMissing.
    let sql = "\
smelt.define session_rollup(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    metrics: SelectItems<Agg, sessionized> = (),
    filters: Expr<Boolean> = TRUE
) -> TableExpr AS (
    WITH sessionized AS (
        SELECT * FROM source
    )
    SELECT user_col, session_id, metrics
    FROM sessionized
    WHERE filters
    GROUP BY user_col, session_id
)
";
    let (sig, select, _body_text) = parse_define_select(sql);

    let mut body_ctx = TypeContext::new();
    // Seed sessionized as a CTE with {user_col: Text, session_id: BigInt}
    body_ctx.add_cte_column(
        "sessionized",
        "user_col",
        TypedColumn::nullable(DataType::Text),
    );
    body_ctx.add_cte_column(
        "sessionized",
        "session_id",
        TypedColumn::nullable(DataType::BigInt),
    );
    // Also seed params in the context
    body_ctx.add_function_param("user_col", TypedColumn::nullable(DataType::Text));
    body_ctx.add_function_param(
        "ts_col",
        TypedColumn::nullable(DataType::Timestamp {
            with_timezone: false,
        }),
    );
    body_ctx.add_function_param("gap", TypedColumn::nullable(DataType::Interval));
    body_ctx.add_function_param("metrics", TypedColumn::nullable(DataType::Unknown));
    body_ctx.add_function_param("filters", TypedColumn::nullable(DataType::Boolean));

    let (arg_expr, arg_range, arg_text) = parse_arg_expr("SUM(session_id)");
    let bindings: HashMap<String, (Expr, TextRange)> =
        HashMap::from([("metrics".to_string(), (arg_expr, arg_range))]);

    let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &bindings, &arg_text);
    let missing: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FragmentColumnMissing))
        .collect();
    assert!(
        missing.is_empty(),
        "`session_id` is in the sessionized CTE — no FragmentColumnMissing expected; got {diags:#?}"
    );
}

// ─── Test 3 ──────────────────────────────────────────────────────────────────

#[test]
fn caller_metrics_fragment_rejects_non_sessionized_column() {
    // Same setup as test 2 but call with SUM(xyz) where xyz is not in
    // the sessionized CTE. Expect FragmentColumnMissing.
    let sql = "\
smelt.define session_rollup(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    metrics: SelectItems<Agg, sessionized> = (),
    filters: Expr<Boolean> = TRUE
) -> TableExpr AS (
    WITH sessionized AS (
        SELECT * FROM source
    )
    SELECT user_col, session_id, metrics
    FROM sessionized
    WHERE filters
    GROUP BY user_col, session_id
)
";
    let (sig, select, _body_text) = parse_define_select(sql);

    let mut body_ctx = TypeContext::new();
    body_ctx.add_cte_column(
        "sessionized",
        "user_col",
        TypedColumn::nullable(DataType::Text),
    );
    body_ctx.add_cte_column(
        "sessionized",
        "session_id",
        TypedColumn::nullable(DataType::BigInt),
    );
    body_ctx.add_function_param("user_col", TypedColumn::nullable(DataType::Text));
    body_ctx.add_function_param(
        "ts_col",
        TypedColumn::nullable(DataType::Timestamp {
            with_timezone: false,
        }),
    );
    body_ctx.add_function_param("gap", TypedColumn::nullable(DataType::Interval));
    body_ctx.add_function_param("metrics", TypedColumn::nullable(DataType::Unknown));
    body_ctx.add_function_param("filters", TypedColumn::nullable(DataType::Boolean));

    let (arg_expr, arg_range, arg_text) = parse_arg_expr("SUM(xyz)");
    let bindings: HashMap<String, (Expr, TextRange)> =
        HashMap::from([("metrics".to_string(), (arg_expr, arg_range))]);

    let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &bindings, &arg_text);
    let missing: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FragmentColumnMissing))
        .collect();
    assert!(
        !missing.is_empty(),
        "`xyz` is not in the sessionized CTE — FragmentColumnMissing expected; got {diags:#?}"
    );
    assert!(
        missing[0].message.contains("xyz"),
        "diagnostic message should mention the column; got {:?}",
        missing[0].message
    );
}

// ─── Test 4 ──────────────────────────────────────────────────────────────────

#[test]
fn empty_default_metrics_splice_comma_elision() {
    // A function with `metrics: SelectItems<Agg, sessionized> = ()` default.
    // file_diagnostics should produce no errors — the type-checker handles
    // the empty default without errors. (SQL comma-elision is deferred to
    // Phase 32/planner; this test only validates the type-checker path.)
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("sr_empty_default.sql");
    let sql = "\
smelt.define session_rollup_empty(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
    metrics: SelectItems<Agg, sessionized> = ()
) -> TableExpr AS (
    WITH sessionized AS (
        SELECT * FROM source
    )
    SELECT user_col, metrics
    FROM sessionized
    GROUP BY user_col
)
";
    let (db, ws, files) = build_db(root, &[(fn_path, sql)]);
    let diags = file_diagnostics(&db, ws, files[0]);

    let unknown_ctx_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownContext))
        .collect();
    assert!(
        unknown_ctx_diags.is_empty(),
        "empty default with CTE context should produce no UnknownContext; got {unknown_ctx_diags:#?}"
    );
}
