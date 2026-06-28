//! Phase 14 TDD tests — `ExprKind` synthesis on parsed SELECT expressions
//! (§16 #24).
//!
//! Companion unit tests for the helpers themselves (`subkind_of`,
//! `kind_ceiling`, registry seeding) live in
//! `crates/smelt-types/src/signatures.rs`. The tests here exercise the
//! end-to-end pure pass: parse SQL → walk the AST → synthesise an
//! [`ExprKind`] alongside the [`DataType`].

use std::path::PathBuf;

use smelt_db::type_inference::{infer_expression_kind, infer_expression_type, TypeContext};
use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};
use smelt_parser::ast::File;
use smelt_types::{DataType, ExprKind, TypedColumn};

/// Parse a `SELECT <expr>` and return the typed result of `<expr>`.
fn infer_one_select_expr(sql: &str, ctx: &TypeContext) -> (TypedColumn, ExprKind) {
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    let select_list = select.select_list().expect("SELECT list");
    let item = select_list.items().next().expect("at least one item");
    let expr = item.expression().expect("item expression");
    let typed = infer_expression_type(&expr, ctx).expect("typed result");
    let kind = infer_expression_kind(&expr, ctx);
    (typed, kind)
}

fn ctx_with_revenue() -> TypeContext {
    let mut ctx = TypeContext::new();
    ctx.add_model_column(
        "orders",
        "revenue",
        TypedColumn::nullable(DataType::Integer),
    );
    ctx.add_model_column("orders", "user_id", TypedColumn::nullable(DataType::BigInt));
    ctx
}

/// `ROW_NUMBER() OVER (...)` synthesises `(BigInt, Window)`.
#[test]
fn window_func_synthesises_window_kind() {
    let ctx = ctx_with_revenue();
    let (typed, kind) = infer_one_select_expr(
        "SELECT ROW_NUMBER() OVER (ORDER BY revenue) AS rn FROM orders",
        &ctx,
    );
    assert_eq!(
        typed.data_type,
        DataType::BigInt,
        "ROW_NUMBER returns BigInt"
    );
    assert_eq!(
        kind,
        ExprKind::Window,
        "ROW_NUMBER() OVER (...) must synthesise Window"
    );
}

/// `SUM(revenue)` (no OVER) synthesises Agg; bare `revenue` synthesises Scalar.
#[test]
fn aggregate_in_select_synthesises_agg_kind() {
    let ctx = ctx_with_revenue();

    // SUM(revenue) → Agg.
    let (_typed, kind) = infer_one_select_expr("SELECT SUM(revenue) AS r FROM orders", &ctx);
    assert_eq!(
        kind,
        ExprKind::Agg,
        "SUM(...) without OVER must synthesise Agg"
    );

    // Bare `revenue` → Scalar.
    let (_typed, kind) = infer_one_select_expr("SELECT revenue FROM orders", &ctx);
    assert_eq!(
        kind,
        ExprKind::Scalar,
        "bare column reference must synthesise Scalar"
    );
}

/// `SUM(revenue) OVER (...)` is the dual-mode case: callee seeded as Agg
/// but the OVER clause lifts the call site to Window.
#[test]
fn aggregate_with_over_clause_becomes_window() {
    let ctx = ctx_with_revenue();
    let (_typed, kind) = infer_one_select_expr(
        "SELECT SUM(revenue) OVER (PARTITION BY user_id) AS r FROM orders",
        &ctx,
    );
    assert_eq!(
        kind,
        ExprKind::Window,
        "SUM(...) OVER (...) must lift the call site to Window"
    );
}

/// Arithmetic + literals + scalar columns stay Scalar.
#[test]
fn arithmetic_of_scalars_is_scalar() {
    let ctx = ctx_with_revenue();
    let (_typed, kind) = infer_one_select_expr("SELECT revenue + 1 AS r FROM orders", &ctx);
    assert_eq!(kind, ExprKind::Scalar);
}

/// Binary expressions take the ceiling of their operands.
#[test]
fn binary_expression_takes_ceiling_of_operands() {
    let ctx = ctx_with_revenue();
    // SUM(revenue) > 100 → max(Agg, Scalar) = Agg.
    let (_typed, kind) = infer_one_select_expr("SELECT SUM(revenue) > 100 AS r FROM orders", &ctx);
    assert_eq!(kind, ExprKind::Agg);

    // ROW_NUMBER() OVER (...) > 1 → max(Window, Scalar) = Window.
    let (_typed, kind) = infer_one_select_expr(
        "SELECT ROW_NUMBER() OVER (ORDER BY revenue) > 1 AS r FROM orders",
        &ctx,
    );
    assert_eq!(kind, ExprKind::Window);
}

// ─── WHERE / GROUP BY rejection (§16 #24) ───────────────────────────────────

/// Build a tiny workspace whose only file is a model containing `sql`.
/// Returns the diagnostics emitted by `file_diagnostics` for that model.
fn diagnostics_for_model(sql: &str) -> Vec<smelt_db::Diagnostic> {
    let mut db = Database::default();
    let project_root = PathBuf::from("/tmp/smelt_phase14_test_project");
    let project = db.set_project_input(project_root.clone(), String::new());
    let model_path = project_root.join("models").join("kind_check.sql");
    let sf: SourceFile = db.set_source_file(model_path, sql.to_string(), project_root);
    db.set_workspace(vec![sf], vec![project]);
    let ws: Workspace = db.workspace();
    file_diagnostics(&db, ws, sf)
}

/// `WHERE ROW_NUMBER() OVER (...) > 1` — the canonical illegal usage.
/// Must produce a `WindowInScalarContext` diagnostic (Phase 14 TDD test 5).
#[test]
fn where_clause_rejects_window_kind() {
    let sql = "SELECT * FROM orders WHERE ROW_NUMBER() OVER (ORDER BY revenue) > 1";
    let diags = diagnostics_for_model(sql);
    let matching: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::WindowInScalarContext))
        .collect();
    assert!(
        !matching.is_empty(),
        "expected at least one WindowInScalarContext diagnostic, got: {diags:#?}"
    );
    let msg = &matching[0].message;
    assert!(
        msg.contains("ROW_NUMBER"),
        "diagnostic must name the offending expression, got: {msg}"
    );
    assert!(
        msg.contains("WHERE"),
        "diagnostic must name the WHERE clause, got: {msg}"
    );
}

/// `GROUP BY ROW_NUMBER() OVER (...)` is also illegal (Phase 14 TDD test 6).
#[test]
fn groupby_rejects_window_kind() {
    let sql = "SELECT user_id FROM orders GROUP BY ROW_NUMBER() OVER (ORDER BY revenue)";
    let diags = diagnostics_for_model(sql);
    let matching: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::WindowInScalarContext))
        .collect();
    assert!(
        !matching.is_empty(),
        "expected at least one WindowInScalarContext diagnostic, got: {diags:#?}"
    );
    let msg = &matching[0].message;
    assert!(
        msg.contains("ROW_NUMBER"),
        "diagnostic must name the offending expression, got: {msg}"
    );
    assert!(
        msg.contains("GROUP BY"),
        "diagnostic must name the GROUP BY clause, got: {msg}"
    );
}

/// Negative control: `WHERE revenue > 100` (a scalar comparison) does NOT
/// produce a WindowInScalarContext diagnostic.
#[test]
fn where_clause_accepts_scalar_kind() {
    let sql = "SELECT * FROM orders WHERE revenue > 100";
    let diags = diagnostics_for_model(sql);
    assert!(
        diags
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::WindowInScalarContext)),
        "scalar WHERE clause must not trigger WindowInScalarContext: {diags:#?}"
    );
}
