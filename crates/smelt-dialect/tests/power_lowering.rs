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
    for (dialect, caps) in [
        (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        (SqlDialect::SparkSQL, BackendCapabilities::spark()),
        (SqlDialect::PostgreSQL, BackendCapabilities::postgresql()),
    ] {
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

/// `//` is lexed and parsed by smelt (`FLOOR_DIVIDE`), but DuckDB's own
/// semantics for it are type-polymorphic in a way that makes an exact
/// GoogleSQL lowering impossible without operand-type information the
/// dialect printer does not have (`PrintContext` carries no type inference).
///
/// Measured directly against DuckDB 1.5.4:
/// - `7 // 2` (both INTEGER) = `3`; `-7 // 2` = `-3`; `7 // -2` = `-3`;
///   `-7 // -2` = `3` — truncating (toward-zero) integer division.
/// - `7.5 // 2` (a DOUBLE operand) = `3.75`; `-7.0 // 2` = `-3.5` — i.e. once
///   either operand is floating point, `//` is *not* division at all, it is
///   literally `/` with no floor or truncation applied.
///
/// GoogleSQL's only floor-division-shaped primitive is `DIV(x, y)`, which
/// requires `INT64`/`NUMERIC` operands, truncates toward zero (matching
/// DuckDB's integer case only), and has no defined behavior for the floating
/// operand case at all. Substituting `DIV` unconditionally would therefore
/// silently compute the wrong answer for float operands (exactly the hazard
/// this task exists to close), and the printer has no static type
/// information to choose correctly between `DIV` and plain `/`. It also has
/// no diagnostic-emission channel (`print` returns a plain `String`, not a
/// `Result`), so a proper compile-time diagnostic cannot be raised from this
/// layer without a larger structural change (tracked as follow-up work, not
/// done here per "no speculative lowerings").
///
/// Leaving `//` unlowered is therefore the correct choice: like `%` before
/// `print_bigquery_modulo` existed, GoogleSQL has no infix `//` either, so an
/// unlowered `//` still fails loud as a BigQuery syntax error — never a
/// silently wrong number.
#[test]
fn no_lowering_registered_for_floor_divide() {
    let sql = "SELECT a // b FROM t";
    let out = bigquery(sql);
    assert_eq!(
        out, sql,
        "`//` is deliberately left unlowered (see doc comment above) — it \
         must still fail loud as a BigQuery syntax error rather than silently \
         approximate: {out}"
    );
}
