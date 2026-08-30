//! GoogleSQL has no infix `%` operator — it spells modulo `MOD(x, y)`. A
//! model body containing `WHERE id % 2 = 0` reaches BigQuery unlowered as a
//! syntax error (measured live, 2026-08-19:
//! `dags_bigquery::diamond_propagation_suffices_on_bigquery`, case 0,
//! `BadRequest: 400 Syntax error: Expected ")" but got "%"`).
//!
//! `%` is DuckDB/PostgreSQL/Spark infix syntax that DuckDB's own printer
//! passes through verbatim (the default path — see `printer.rs`'s
//! `SyntaxKind::BINARY_EXPR` fallthrough). The dialect printer must lower it
//! for BigQuery, the same way it already lowers `MEDIAN`
//! (`print_bigquery_median`) and `QUALIFY`/`ARRAY[...]`/etc: a capability gap
//! is a lowering instruction, never a reason to emit invalid SQL
//! (`docs/specs/multi_backend.md` §"Parity contract").

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::parse;

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

fn bigquery(sql: &str) -> String {
    print_with(sql, &SqlDialect::BigQuery, &BackendCapabilities::bigquery())
}

#[test]
fn bigquery_lowers_infix_modulo_to_mod_call() {
    let out = bigquery("SELECT * FROM events WHERE id % 2 = 0");
    assert!(
        !out.contains('%'),
        "GoogleSQL has no infix `%` — it must not survive into the printed SQL: {out}"
    );
    assert!(
        out.contains("MOD(id, 2)"),
        "infix `%` must lower to a MOD(...) call: {out}"
    );
}

#[test]
fn bigquery_lowers_modulo_over_expression_operands() {
    let out = bigquery("SELECT (a + 1) % (b - 1) AS m FROM t");
    assert!(
        out.contains("MOD(a + 1, b - 1)"),
        "operand expressions are passed through, not just bare columns: {out}"
    );
}

#[test]
fn the_lowered_modulo_parses_back_cleanly() {
    let out = bigquery("SELECT * FROM events WHERE id % 2 = 0");
    let parsed = parse(&out);
    assert!(
        parsed.errors.is_empty(),
        "{out}\nmust parse back cleanly, got: {:?}",
        parsed.errors
    );
}

#[test]
fn other_dialects_keep_infix_modulo_verbatim() {
    for (dialect, caps) in [
        (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        (SqlDialect::SparkSQL, BackendCapabilities::spark()),
        (SqlDialect::PostgreSQL, BackendCapabilities::postgresql()),
    ] {
        let sql = "SELECT * FROM events WHERE id % 2 = 0";
        let out = print_with(sql, &dialect, &caps);
        assert_eq!(out, sql, "{} must print `%` unchanged", dialect.name());
    }
}
