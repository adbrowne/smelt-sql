//! Phase 2 TDD tests for pipe SQL passthrough lowering.
//!
//! Tests that contiguous passthrough stages in a pipe query collapse into a
//! single standard SELECT when the backend has `supports_pipe_syntax = false`.

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::parse;

fn duckdb_ctx() -> (SqlDialect, BackendCapabilities) {
    (SqlDialect::DuckDB, BackendCapabilities::duckdb())
}

fn print_with(sql: &str, dialect: &SqlDialect, caps: &BackendCapabilities, schema: &str) -> String {
    let parsed = parse(sql);
    let ctx = PrintContext {
        dialect,
        capabilities: caps,
        schema,
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
    };
    print(&parsed.syntax(), &ctx)
}

// ── Test 1: passthrough stages collapse to one SELECT ───────────────────────

/// A pipe query with FROM, WHERE, SELECT, ORDER BY, and LIMIT stages must
/// collapse to a single standard SELECT. No `|>` token may appear in the output.
#[test]
fn passthrough_collapses_to_one_select() {
    let sql = "FROM t |> WHERE a > 0 |> SELECT a, b |> ORDER BY a |> LIMIT 5";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");

    // No pipe operator token in output
    assert!(
        !result.contains("|>"),
        "output must not contain |>, got: {result}"
    );

    // Must contain core clauses
    assert!(
        result.contains("SELECT a, b"),
        "expected SELECT a, b in output, got: {result}"
    );
    assert!(
        result.contains("FROM t"),
        "expected FROM t in output, got: {result}"
    );
    assert!(
        result.contains("WHERE a > 0"),
        "expected WHERE a > 0 in output, got: {result}"
    );
    assert!(
        result.contains("ORDER BY a"),
        "expected ORDER BY a in output, got: {result}"
    );
    assert!(
        result.contains("LIMIT 5"),
        "expected LIMIT 5 in output, got: {result}"
    );
}

// ── Test 2: DISTINCT lowers ──────────────────────────────────────────────────

/// A pipe query with SELECT followed by DISTINCT must produce SELECT DISTINCT.
#[test]
fn distinct_lowers() {
    let sql = "FROM t |> SELECT a |> DISTINCT";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");

    assert!(
        !result.contains("|>"),
        "output must not contain |>, got: {result}"
    );
    assert!(
        result.contains("DISTINCT"),
        "expected DISTINCT in output, got: {result}"
    );
    assert!(
        result.contains("SELECT"),
        "expected SELECT in output, got: {result}"
    );
    assert!(
        result.contains("FROM t"),
        "expected FROM t in output, got: {result}"
    );
}

// ── Test 3: FROM-only pipe query (no stages) ─────────────────────────────────

/// A bare `FROM t` with no stages (no `|>`) is not a PIPE_QUERY — but
/// `FROM t` *with* at least one pipe stage is. Verify from-only (minimal pipe).
#[test]
fn from_with_single_where_stage() {
    let sql = "FROM orders |> WHERE status = 'paid'";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");

    assert!(
        !result.contains("|>"),
        "output must not contain |>, got: {result}"
    );
    assert!(
        result.contains("FROM orders"),
        "expected FROM orders in output, got: {result}"
    );
    assert!(
        result.contains("status = 'paid'"),
        "expected WHERE predicate in output, got: {result}"
    );
    // When no explicit SELECT stage, select list defaults to *
    assert!(
        result.contains("SELECT"),
        "expected SELECT in output, got: {result}"
    );
}

// ── Test 4: no select stage → implicit SELECT * ───────────────────────────────

/// When there is no `|> SELECT` stage, the emitted select list is `*`.
#[test]
fn no_select_stage_implies_star() {
    let sql = "FROM t |> WHERE x = 1 |> LIMIT 10";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");

    assert!(
        !result.contains("|>"),
        "output must not contain |>, got: {result}"
    );
    assert!(
        result.contains("SELECT *"),
        "expected SELECT * when no |> SELECT stage, got: {result}"
    );
    assert!(result.contains("FROM t"), "expected FROM t, got: {result}");
    assert!(
        result.contains("WHERE x = 1"),
        "expected WHERE x = 1, got: {result}"
    );
    assert!(
        result.contains("LIMIT 10"),
        "expected LIMIT 10, got: {result}"
    );
}

// ── Test 5: WHERE after SELECT is non-passthrough ────────────────────────────

/// A `|> WHERE` that follows a `|> SELECT` must NOT be collapsed into the same
/// WHERE clause (alias from SELECT is not visible in WHERE in standard SQL).
/// The query must be treated as non-passthrough (prints verbatim for now).
#[test]
fn where_after_select_is_non_passthrough() {
    let sql = "FROM t |> SELECT a AS x |> WHERE x > 0";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    // This should NOT produce "SELECT a AS x FROM t WHERE x > 0".
    // Until Phase 3 adds subquery-wrap lowering, this falls back to verbatim.
    // The important thing: it must not produce a single-level WHERE with a SELECT alias.
    assert!(
        !result.contains("FROM t WHERE x > 0"),
        "WHERE after SELECT must not collapse into single-level WHERE, got: {result}"
    );
}

// ── Test 5 (Phase 3): EXTEND wraps prior projection ──────────────────────────

/// After a SELECT that has already fixed the projection, an EXTEND must wrap
/// the prior query as a subquery.
///
/// Input: `FROM t |> SELECT a |> EXTEND a + 1 AS b`
/// Expected lowered form: `SELECT *, a + 1 AS b FROM (SELECT a FROM t)`
/// No `|>` in output.
#[test]
fn extend_wraps_prior_projection() {
    let sql = "FROM t |> SELECT a |> EXTEND a + 1 AS b";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");

    assert!(
        !result.contains("|>"),
        "output must not contain |>, got: {result}"
    );

    // The result must be a subquery wrapping the SELECT a FROM t
    assert!(
        result.contains("SELECT a FROM t") || result.contains("SELECT a\nFROM t"),
        "expected SELECT a FROM t as a subquery, got: {result}"
    );

    // The EXTEND expression must appear in the outer SELECT
    assert!(
        result.contains("a + 1") && result.contains("AS b"),
        "expected a + 1 AS b in output, got: {result}"
    );

    // The outer SELECT must project *
    assert!(
        result.contains("SELECT *") || result.contains("SELECT *, "),
        "expected SELECT * or SELECT *, in output, got: {result}"
    );
}

// ── Test 6: no pipe token reaches backend ────────────────────────────────────

/// All backends report supports_pipe_syntax = false; none may emit `|>`.
#[test]
fn no_pipe_token_reaches_backend() {
    let sql = "FROM orders |> WHERE status = 'paid' |> SELECT customer_id, amount |> ORDER BY amount DESC |> LIMIT 100";

    let backends = [
        (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        (SqlDialect::SparkSQL, BackendCapabilities::spark_delta()),
        (SqlDialect::PostgreSQL, BackendCapabilities::postgresql()),
    ];

    for (dialect, caps) in &backends {
        let result = print_with(sql, dialect, caps, "main");
        assert!(
            !result.contains("|>"),
            "backend {:?} must not emit |>, got: {result}",
            dialect
        );
    }
}
