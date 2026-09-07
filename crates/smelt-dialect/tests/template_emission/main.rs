//! `Emission::Template` — the generic printer interpreter that replaced the
//! hand-written `RewriteId::ModuloCall`/`RewriteId::PowerCall` rewrites.
//!
//! [`pinned_output::modulo_and_power_output_is_pinned`] asserts the printed
//! SQL for `%`, `^` and `**` is byte-identical to the pre-migration output
//! (captured from the `print_modulo_call`/`print_power_call` implementation
//! this phase retired). `pinned_output`'s other two tests exercise the
//! interpreter directly, independent of any registry row, since no
//! currently-registered template is non-call-shaped. [`date_functions`]
//! exercises the real `print` entry point for the `DATE_ADD`/`DATE_SUB`
//! template rows.

mod date_functions;
mod pinned_output;

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, print_template, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::syntax_kind::SyntaxKind;
use smelt_parser::{parse, FunctionCall};

pub(crate) fn print_with(sql: &str, dialect: &SqlDialect, caps: &BackendCapabilities) -> String {
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
        settled_emissions: &[],
    };
    print(&parsed.syntax(), &ctx)
}

pub(crate) fn duckdb(sql: &str) -> String {
    print_with(sql, &SqlDialect::DuckDB, &BackendCapabilities::duckdb())
}

pub(crate) fn spark(sql: &str) -> String {
    print_with(sql, &SqlDialect::SparkSQL, &BackendCapabilities::spark())
}

pub(crate) fn bigquery(sql: &str) -> String {
    print_with(sql, &SqlDialect::BigQuery, &BackendCapabilities::bigquery())
}

/// A synthetic, registry-independent exercise of the interpreter: parse a
/// standalone expression, take its top-level node as a positional argument,
/// and check `print_template`'s parenthesisation decision by node kind alone.
pub(crate) fn first_arg_node(sql: &str) -> smelt_parser::syntax_kind::SyntaxNode {
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

pub(crate) fn print_template_str(
    template: &str,
    args: &[smelt_parser::syntax_kind::SyntaxNode],
) -> String {
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
        settled_emissions: &[],
    };
    // The template's own node identity doesn't matter for a synthetic,
    // registry-independent call — reuse the first argument's node as a stand-in
    // so `print_template` has something to anchor trailing-trivia lookup on.
    let mut out = String::new();
    print_template(&args[0], template, args, &ctx, &mut out);
    out
}
