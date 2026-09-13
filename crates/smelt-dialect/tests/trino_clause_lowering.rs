//! Trino's clause-shaped divergences (`QUALIFY`, `::`, trailing commas,
//! `[a, b]` array literals) are already `BackendCapabilities`-flag-driven —
//! `trino_iceberg()` sets `supports_qualify: false`, `supports_double_colon_cast:
//! false`, `supports_trailing_commas: false`, `supports_array_literal: true` —
//! so the printer's existing generic dispatch (`printer/mod.rs`) needs no Trino
//! branch of its own. These tests prove the flags actually drive Trino's
//! lowering, the same way `power_lowering.rs`/`modulo_lowering.rs` prove it for
//! the operator divergences (`docs/outcomes/20260913-trino-emission` phase 3).

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, unsupported_emissions, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::parse;
use smelt_types::DialectId;

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
        settled_emissions: &[],
    };
    print(&parsed.syntax(), &ctx)
}

fn trino(sql: &str) -> String {
    print_with(
        sql,
        &SqlDialect::Trino,
        &BackendCapabilities::trino_iceberg(),
    )
}

/// `mismatched input 'QUALIFY'` on a live coordinator — Trino has no `QUALIFY`
/// clause, so it takes the same subquery + outer `WHERE` rewrite Spark and
/// Databricks already use.
#[test]
fn qualify_is_rewritten_to_an_outer_subquery() {
    let sql = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
    let out = trino(sql);
    assert!(
        out.contains("SELECT * FROM ("),
        "expected a subquery wrapper, got: {out}"
    );
    assert!(
        out.contains("WHERE rn = 1"),
        "expected the QUALIFY predicate hoisted into an outer WHERE: {out}"
    );
    assert!(!out.contains("QUALIFY"), "QUALIFY must not survive: {out}");
}

/// `SELECT 1::INTEGER` fails to parse on Trino — no `::` operator — so it
/// takes the same `CAST(expr AS type)` rewrite GoogleSQL and Spark use.
#[test]
fn double_colon_cast_is_rewritten_to_cast_call() {
    let out = trino("SELECT x::INTEGER FROM t");
    assert_eq!(out, "SELECT CAST(x AS INTEGER) FROM t");
}

/// A trailing comma before `<EOF>`/`FROM`/etc. fails to parse on Trino, so it
/// is stripped, the same way it is for Spark and Databricks.
#[test]
fn trailing_commas_are_dropped() {
    let out = trino("SELECT a, b, FROM t");
    assert!(
        !out.contains("b,"),
        "trailing comma should be removed: {out}"
    );
    assert!(out.contains("a, b"), "non-trailing commas preserved: {out}");
    assert!(!out.contains(", FROM"), "comma before FROM removed: {out}");
}

/// Trino is the first non-DuckDB dialect where `SELECT [1,2,3]` executes
/// verbatim (measured live) — `supports_array_literal: true` — so, unlike
/// Spark's `ARRAY(...)` rewrite, the bracket literal must survive unchanged.
#[test]
fn array_literal_brackets_are_kept_native() {
    let sql = "SELECT ARRAY[1, 2, 3] FROM t";
    let out = trino(sql);
    assert_eq!(out, sql, "Trino must print array literals unchanged");
}

/// `PIVOT (COUNT(id) FOR cat IN ('a'))` executes cleanly against a live
/// Trino coordinator (phase 4 of `docs/outcomes/20260913-trino-emission`) —
/// unlike `UNPIVOT`, Trino's grammar does have `PIVOT`, so it must not be
/// refused at the dialect layer.
#[test]
fn trino_keeps_pivot_native() {
    let sql = "SELECT * FROM t PIVOT (COUNT(id) FOR cat IN ('a'))";
    let found = unsupported_emissions(&parse(sql).syntax(), SqlDialect::Trino, |_| None);
    assert!(
        found.is_empty(),
        "PIVOT is native on Trino and must not be refused: {found:#?}"
    );
}

/// `UNPIVOT (val FOR name IN (a, b, c))` fails to parse against a live Trino
/// coordinator: `mismatched input 'UNPIVOT'` — Trino's grammar has no
/// `UNPIVOT` keyword at all, so the dialect layer refuses it as a backstop
/// for `compile_with_sql`, which runs no diagnostics query.
#[test]
fn trino_refuses_unpivot_clause() {
    let sql = "SELECT * FROM t UNPIVOT (val FOR name IN (a, b, c))";
    let found = unsupported_emissions(&parse(sql).syntax(), SqlDialect::Trino, |_| None);
    assert_eq!(found.len(), 1, "expected exactly one refusal: {found:#?}");
    assert_eq!(found[0].name, "UNPIVOT");
    assert_eq!(found[0].dialect, DialectId::Trino);
}

/// The same `UNPIVOT` tree on DuckDB is clean — the flag is consulted, not
/// hardwired to a specific dialect.
#[test]
fn duckdb_keeps_unpivot_native() {
    let sql = "SELECT * FROM t UNPIVOT (val FOR name IN (a, b, c))";
    let found = unsupported_emissions(&parse(sql).syntax(), SqlDialect::DuckDB, |_| None);
    assert!(
        found.is_empty(),
        "UNPIVOT is native on DuckDB and must not be refused: {found:#?}"
    );
}
