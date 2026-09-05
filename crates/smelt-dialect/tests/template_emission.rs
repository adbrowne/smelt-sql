//! `Emission::Template` — the generic printer interpreter that replaced the
//! hand-written `RewriteId::ModuloCall`/`RewriteId::PowerCall` rewrites.
//!
//! `modulo_and_power_output_is_pinned` asserts the printed SQL for `%`, `^`
//! and `**` is byte-identical to the pre-migration output (captured from the
//! `print_modulo_call`/`print_power_call` implementation this phase retired).
//! `compound_argument_is_parenthesised` and `non_call_template_is_wrapped_in_parens`
//! exercise the interpreter directly, independent of any registry row, since no
//! currently-registered template is non-call-shaped.

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, print_template, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::syntax_kind::SyntaxKind;
use smelt_parser::{parse, FunctionCall};

fn print_with(sql: &str, dialect: &SqlDialect, caps: &BackendCapabilities) -> String {
    let parsed = parse(sql);
    let ctx = PrintContext {
        dialect,
        capabilities: caps,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
    };
    print(&parsed.syntax(), &ctx)
}

fn duckdb(sql: &str) -> String {
    print_with(sql, &SqlDialect::DuckDB, &BackendCapabilities::duckdb())
}

fn spark(sql: &str) -> String {
    print_with(sql, &SqlDialect::SparkSQL, &BackendCapabilities::spark())
}

fn bigquery(sql: &str) -> String {
    print_with(sql, &SqlDialect::BigQuery, &BackendCapabilities::bigquery())
}

/// (sql, dialect-printer, expected).
type PinnedCase = (&'static str, fn(&str) -> String, &'static str);

#[test]
fn modulo_and_power_output_is_pinned() {
    // Expected strings captured from the pre-migration
    // `print_modulo_call`/`print_power_call` implementation.
    let cases: &[PinnedCase] = &[
        // Bare columns.
        ("SELECT a % b FROM t", bigquery, "SELECT MOD(a, b) FROM t"),
        ("SELECT a % b FROM t", duckdb, "SELECT a % b FROM t"),
        ("SELECT a % b FROM t", spark, "SELECT a % b FROM t"),
        ("SELECT a ^ b FROM t", bigquery, "SELECT POWER(a, b) FROM t"),
        ("SELECT a ^ b FROM t", spark, "SELECT POWER(a, b) FROM t"),
        ("SELECT a ^ b FROM t", duckdb, "SELECT a ^ b FROM t"),
        (
            "SELECT a ** b FROM t",
            bigquery,
            "SELECT POWER(a, b) FROM t",
        ),
        ("SELECT a ** b FROM t", spark, "SELECT POWER(a, b) FROM t"),
        ("SELECT a ** b FROM t", duckdb, "SELECT a ** b FROM t"),
        // Literals.
        ("SELECT 10 % 3 FROM t", bigquery, "SELECT MOD(10, 3) FROM t"),
        // Nested calls as operands.
        (
            "SELECT f(a) % g(b) FROM t",
            bigquery,
            "SELECT MOD(f(a), g(b)) FROM t",
        ),
        // Parenthesised compound operands — parens are dropped, not
        // reproduced, exactly like the pre-migration rewrite
        // (`modulo_lowering.rs::bigquery_lowers_modulo_over_expression_operands`).
        (
            "SELECT (a + 1) % (b - 1) FROM t",
            bigquery,
            "SELECT MOD(a + 1, b - 1) FROM t",
        ),
        (
            "SELECT (a + 1) ^ (b - 1) FROM t",
            bigquery,
            "SELECT POWER(a + 1, b - 1) FROM t",
        ),
        // Nested same-operator chain — left-associative, so the left operand
        // is itself a lowered call, recursively dispatched through
        // `print_node`.
        (
            "SELECT a % b % c FROM t",
            bigquery,
            "SELECT MOD(MOD(a, b), c) FROM t",
        ),
        (
            "SELECT a ^ b ^ c FROM t",
            bigquery,
            "SELECT POWER(POWER(a, b), c) FROM t",
        ),
    ];
    for (sql, printer, expected) in cases {
        let out = printer(sql);
        assert_eq!(&out, expected, "input: {sql}");
    }
}

/// A synthetic, registry-independent exercise of the interpreter: parse a
/// standalone expression, take its top-level node as a positional argument,
/// and check `print_template`'s parenthesisation decision by node kind alone.
fn first_arg_node(sql: &str) -> smelt_parser::syntax_kind::SyntaxNode {
    let parsed = parse(sql);
    let root = parsed.syntax();
    let call = root
        .descendants()
        .find(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
        .expect("expected a FUNCTION_CALL in the fixture");
    FunctionCall::cast(call)
        .expect("cast")
        .arguments()
        .into_iter()
        .next()
        .expect("expected at least one argument")
        .syntax()
        .clone()
}

fn print_template_str(template: &str, args: &[smelt_parser::syntax_kind::SyntaxNode]) -> String {
    let caps = BackendCapabilities::duckdb();
    let ctx = PrintContext {
        dialect: &SqlDialect::DuckDB,
        capabilities: &caps,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
    };
    // The template's own node identity doesn't matter for a synthetic,
    // registry-independent call — reuse the first argument's node as a stand-in
    // so `print_template` has something to anchor trailing-trivia lookup on.
    let mut out = String::new();
    print_template(&args[0], template, args, &ctx, &mut out);
    out
}

#[test]
fn compound_argument_is_parenthesised() {
    // `{0} - 1` is itself non-call-shaped, so its whole output is wrapped
    // once regardless of the argument (`non_call_template_is_wrapped_in_parens`
    // covers that rule in isolation); the assertions below are about the
    // *inner* pair — present only when the argument itself is compound.

    // `f(a + b)` — the argument is a bare (unparenthesised in source)
    // BINARY_EXPR: compound, so it picks up its own inner wrap on top of the
    // template's outer one.
    let compound = first_arg_node("SELECT f(a + b) FROM t");
    let out = print_template_str("{0} - 1", &[compound]);
    assert_eq!(out, "((a + b) - 1)");

    // `f((a + b))` — a parenthesised compound. The grammar drops the literal
    // source parens once already reaching this accessor (the parenthesised
    // group's own `EXPRESSION` wrapper is what `arguments()` returns, and it
    // spans just the inner content), so this prints identically to the
    // unparenthesised case above — nothing here to double-wrap.
    let paren_compound = first_arg_node("SELECT f((a + b)) FROM t");
    let out = print_template_str("{0} - 1", &[paren_compound]);
    assert_eq!(out, "((a + b) - 1)");

    // `f(x)` — a bare identifier is an atom: no inner wrap, only the
    // template's own outer one.
    let ident = first_arg_node("SELECT f(x) FROM t");
    let out = print_template_str("{0} - 1", &[ident]);
    assert_eq!(out, "(x - 1)");
}

#[test]
fn non_call_template_is_wrapped_in_parens() {
    // A template such as `{0} - {1}` is not call-shaped — its output must be
    // wrapped so it composes correctly inside a larger expression.
    let left = first_arg_node("SELECT f(a, b) FROM t");
    let call = parse("SELECT f(a, b) FROM t").syntax();
    let fc = call
        .descendants()
        .find(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
        .unwrap();
    let args = FunctionCall::cast(fc)
        .unwrap()
        .arguments()
        .into_iter()
        .map(|e| e.syntax().clone())
        .collect::<Vec<_>>();
    let out = print_template_str("{0} - {1}", &args);
    assert_eq!(out, "(a - b)");
    // Sanity: the call-shaped counterpart never wraps.
    let out_call_shaped = print_template_str("MOD({0}, {1})", &args);
    assert_eq!(out_call_shaped, "MOD(a, b)");
    let _ = left;
}
