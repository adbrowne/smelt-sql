//! GoogleSQL's infix `^` is bitwise XOR, not exponentiation — a strictly more
//! dangerous gap than the `%` syntax error `modulo_lowering.rs` closes,
//! because `val ^ 2` reaches BigQuery, parses, and *silently computes the
//! wrong number* instead of failing loud. smelt lexes `^` as `CARET`,
//! documented in `crates/smelt-parser/src/syntax_kind.rs` as "power, synonym
//! for `**`" (DuckDB semantics) — measured directly against DuckDB 1.5.4:
//! `2 ^ 10` and `2 ** 10` and `power(2, 10)` all produce `1024.0` (DOUBLE),
//! and this holds across negative bases (`(-2) ^ 3 = -8.0`), negative
//! exponents (`2 ^ (-1) = 0.5`), fractional exponents (`2.0 ^ 0.5 =
//! 1.4142135623730951`), and integer-typed operands (`2::int ^ 3::int` is
//! still `DOUBLE`, matching bare `power(2, 3)`) — so `^` and `**` are
//! DuckDB-exact synonyms for `POWER(x, y)` in every case tested, and GoogleSQL
//! `POWER` likewise always returns `FLOAT64`. The one measured divergence is
//! `0 ^ (-1)`: DuckDB returns `inf`, while GoogleSQL's `POWER` raises a
//! runtime "invalid argument" error per SQL:2003 — a *loud* failure on
//! BigQuery, not a silent one, so it does not block this lowering (recorded
//! as a known divergence in the report, not silently swallowed).
//!
//! `//` (floor division) is deliberately NOT lowered here — see
//! `no_lowering_registered_for_floor_divide` below for why.

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::parse;
use smelt_types::{BuiltinRegistry, Emission};

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
fn bigquery_lowers_infix_caret_to_power_call() {
    let out = bigquery("SELECT * FROM events WHERE val ^ 2 = 4");
    assert!(
        !out.contains('^'),
        "GoogleSQL `^` is bitwise XOR, not power — it must never survive into \
         the printed SQL unlowered: {out}"
    );
    assert!(
        out.contains("POWER(val, 2)"),
        "infix `^` must lower to a POWER(...) call: {out}"
    );
}

#[test]
fn bigquery_lowers_infix_double_star_to_power_call() {
    let out = bigquery("SELECT val ** 2 AS squared FROM events");
    assert!(
        out.contains("POWER(val, 2)"),
        "infix `**` must lower to a POWER(...) call: {out}"
    );
}

#[test]
fn bigquery_lowers_power_over_expression_operands() {
    let out = bigquery("SELECT (a + 1) ^ (b - 1) AS m FROM t");
    assert!(
        out.contains("POWER(a + 1, b - 1)"),
        "operand expressions are passed through, not just bare columns: {out}"
    );
}

#[test]
fn the_lowered_power_parses_back_cleanly() {
    let out = bigquery("SELECT * FROM events WHERE val ^ 2 = 4");
    let parsed = parse(&out);
    assert!(
        parsed.errors.is_empty(),
        "{out}\nmust parse back cleanly, got: {:?}",
        parsed.errors
    );
}

#[test]
fn other_dialects_keep_infix_caret_and_double_star_verbatim() {
    // DuckDB has no XOR hazard for `^` — it treats it as power. No rewrite
    // needed; verbatim emission is correct.
    for (dialect, caps) in [(SqlDialect::DuckDB, BackendCapabilities::duckdb())] {
        for sql in ["SELECT val ^ 2 FROM t", "SELECT val ** 2 FROM t"] {
            let out = print_with(sql, &dialect, &caps);
            assert_eq!(
                out,
                sql,
                "{} must print power operators unchanged",
                dialect.name()
            );
        }
    }
}

/// Spark's `^` is bitwise XOR, not exponentiation — the same silent-wrong-answer
/// hazard as BigQuery. The registry maps `^` and `**` to `RewriteId::PowerCall`
/// for Spark, so the printer must lower them to `POWER(...)`.
#[test]
fn spark_lowers_infix_caret_to_power_call() {
    let (dialect, caps) = (SqlDialect::SparkSQL, BackendCapabilities::spark());
    let out = print_with("SELECT a ^ b FROM t", &dialect, &caps);
    assert_eq!(
        out, "SELECT POWER(a, b) FROM t",
        "Spark `^` is bitwise XOR, not power — must lower to POWER(...): {out}"
    );
}

/// `//` (floor divide) is declared `Unsupported` in the registry for Spark and
/// BigQuery. The printer still emits `//` verbatim; the compile
/// path owns the refusal (`UnsupportedOnBackend`). The unsupported verdict is
/// asserted here as a registry fact so it is not silently dropped.
///
/// DuckDB's `//` semantics are type-polymorphic (`7 // 2 = 3` integer division,
/// `7.5 // 2 = 3.75` floating-point division) in a way that makes a portable
/// lowering impossible without operand-type information.  No rewrite exists that
/// maps correctly across all operand types, so the registry records
/// `Unsupported` rather than a wrong approximation.
#[test]
fn floor_divide_is_declared_unsupported_rather_than_lowered() {
    let sql = "SELECT a // b FROM t";
    // Printer still emits verbatim — the refusal lives in the compile path.
    let out = bigquery(sql);
    assert_eq!(
        out, sql,
        "`//` must reach the warehouse verbatim so it fails loud rather than \
         silently approximates: {out}"
    );

    // Registry verdict: Unsupported on Spark and BigQuery.
    let sig = BuiltinRegistry::resolve("//").expect("floor-divide `//` must have a registry entry");
    for dialect in [SqlDialect::SparkSQL, SqlDialect::BigQuery] {
        let emission = sig.emission_at(dialect.id(), smelt_types::signatures::Position::Any);
        assert!(
            matches!(emission, Emission::Unsupported { .. }),
            "floor-divide `//` must be Unsupported on {}, got {:?}",
            dialect.name(),
            emission
        );
    }
}
