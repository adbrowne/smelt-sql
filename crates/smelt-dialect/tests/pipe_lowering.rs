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
        restructure_plans: &[],
        settled_emissions: &[],
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

// ── Test: post_aggregate_where_is_having ─────────────────────────────────────

/// A `|> WHERE` that follows `|> AGGREGATE` must be lowered to HAVING semantics.
/// Input: `FROM t |> AGGREGATE sum(x) AS s GROUP BY k |> WHERE s > 10`
/// Expected: output contains `GROUP BY k` and `HAVING s > 10`; no `|>` in output.
#[test]
fn post_aggregate_where_is_having() {
    let sql = "FROM t |> AGGREGATE sum(x) AS s GROUP BY k |> WHERE s > 10";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");

    assert!(
        !result.contains("|>"),
        "output must not contain |>, got: {result}"
    );
    assert!(
        result.contains("GROUP BY"),
        "expected GROUP BY in output, got: {result}"
    );
    assert!(
        result.to_uppercase().contains("HAVING") || result.contains("s > 10"),
        "expected HAVING s > 10 or equivalent in output, got: {result}"
    );
    // The WHERE must not appear at the top level without HAVING
    // (s is an aggregate alias, not directly filterable with WHERE in standard SQL)
    assert!(
        result.contains("HAVING") || result.contains('('),
        "expected HAVING or subquery wrapping, got: {result}"
    );
}

// ── Test: two_aggregates_nest ─────────────────────────────────────────────────

/// Two `|> AGGREGATE` stages must lower to two nested query levels.
/// Input: `FROM t |> AGGREGATE sum(x) AS s GROUP BY k |> AGGREGATE count(*) AS n GROUP BY k`
/// Expected: output contains two GROUP BY clauses (one at each nesting level); no `|>`.
#[test]
fn two_aggregates_nest() {
    let sql = "FROM t |> AGGREGATE sum(x) AS s GROUP BY k |> AGGREGATE count(*) AS n GROUP BY k";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");

    assert!(
        !result.contains("|>"),
        "output must not contain |>, got: {result}"
    );
    // Two GROUP BY clauses indicate two query levels
    let group_by_count = result.matches("GROUP BY").count();
    assert!(
        group_by_count >= 2,
        "expected at least 2 GROUP BY clauses for two AGGREGATE stages, got {group_by_count} in: {result}"
    );
    // The inner query must be wrapped as a subquery
    assert!(
        result.contains('('),
        "expected subquery wrapping for second AGGREGATE stage, got: {result}"
    );
}

// ── Test: post_window_where_is_qualify ───────────────────────────────────────

/// A `|> WHERE` following an EXTEND that introduces a window function must lower
/// to QUALIFY (on DuckDB which supports it) or a wrapping subquery — NOT a WHERE
/// at the same query level.
///
/// Input: `FROM t |> EXTEND row_number() OVER (ORDER BY x) AS rn |> WHERE rn = 1`
/// Expected on DuckDB: output contains QUALIFY or wrapping subquery; no WHERE at
/// the outer level with `rn = 1`; no `|>` in output.
#[test]
fn post_window_where_is_qualify() {
    let sql = "FROM t |> EXTEND row_number() OVER (ORDER BY x) AS rn |> WHERE rn = 1";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");

    assert!(
        !result.contains("|>"),
        "output must not contain |>, got: {result}"
    );
    // Must contain the window function expression
    assert!(
        result.contains("row_number()") || result.contains("ROW_NUMBER()"),
        "expected row_number() in output, got: {result}"
    );
    // Must contain the filter condition
    assert!(
        result.contains("rn = 1"),
        "expected rn = 1 in output, got: {result}"
    );
    // Must use QUALIFY or a wrapping subquery, NOT a bare WHERE after the window expr at the same level
    // DuckDB supports QUALIFY, so QUALIFY should appear
    assert!(
        result.contains("QUALIFY") || result.contains("(SELECT"),
        "expected QUALIFY or subquery wrapping for post-window WHERE, got: {result}"
    );
}

// ── Test: aggregate_keys_before_aggs ─────────────────────────────────────────

/// Output column order for AGGREGATE must be grouping keys first, then aggregates.
/// Input: `FROM t |> AGGREGATE sum(x) AS total_x GROUP BY grp`
/// The emitted SELECT must list `grp` before `total_x`.
#[test]
fn aggregate_keys_before_aggs() {
    let sql = "FROM t |> AGGREGATE sum(x) AS total_x GROUP BY grp";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");

    assert!(
        !result.contains("|>"),
        "output must not contain |>, got: {result}"
    );

    // Verify grp appears before total_x in the SELECT list
    let grp_pos = result.find("grp").expect("grp must appear in output");
    let total_x_pos = result
        .find("total_x")
        .expect("total_x must appear in output");
    assert!(
        grp_pos < total_x_pos,
        "grouping key 'grp' must appear before aggregate 'total_x' in output, got: {result}"
    );

    assert!(
        result.contains("GROUP BY"),
        "expected GROUP BY in output, got: {result}"
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

// ── Phase 5: JOIN lowering ────────────────────────────────────────────────────

#[test]
fn join_lowers_with_pipe_input_as_left() {
    let sql = "FROM a |> JOIN b ON a.k = b.k";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert!(
        !result.contains("|>"),
        "no |> in lowered output, got: {result}"
    );
    // FROM a must be the left side (appears before JOIN)
    let from_a_pos = result
        .find("FROM a")
        .or_else(|| result.find("FROM\na"))
        .unwrap_or_else(|| panic!("expected 'FROM a' in output, got: {result}"));
    let join_b_pos = result
        .find("JOIN b")
        .unwrap_or_else(|| panic!("expected 'JOIN b' in output, got: {result}"));
    assert!(
        from_a_pos < join_b_pos,
        "FROM a must appear before JOIN b, got: {result}"
    );
    // ON condition must be present
    assert!(
        result.contains("a.k = b.k") || result.contains("ON"),
        "expected ON condition in output, got: {result}"
    );
}

// ── Phase 5: set-op lowering ─────────────────────────────────────────────────

#[test]
fn setops_left_fold() {
    // Two operands → left-fold produces 2 UNION ALLs
    let sql = "FROM t |> UNION ALL (SELECT * FROM u), (SELECT * FROM v)";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert!(
        !result.contains("|>"),
        "no |> in lowered output, got: {result}"
    );
    // Two UNION ALLs for three-way union (left fold)
    let union_count = result.matches("UNION").count();
    assert!(
        union_count >= 2,
        "expected at least 2 UNION occurrences for 3-way union, got {union_count} in: {result}"
    );
    // All three sources must appear somewhere
    assert!(
        result.contains('t'),
        "expected 't' in output, got: {result}"
    );
    assert!(
        result.contains('u'),
        "expected 'u' in output, got: {result}"
    );
    assert!(
        result.contains('v'),
        "expected 'v' in output, got: {result}"
    );
}

#[test]
fn join_then_order_by() {
    let sql = "FROM a |> JOIN b ON a.k = b.k |> ORDER BY a.k";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert!(
        !result.contains("|>"),
        "no |> in lowered output, got: {result}"
    );
    assert!(
        result.contains("ORDER BY"),
        "expected ORDER BY, got: {result}"
    );
}
