//! Placement integration tests for pipe SQL.
//!
//! Verifies that a pipe query works in every position the spec allows:
//! - as a subquery body inside a parenthesised FROM expression,
//! - as a CTE body inside a WITH clause.
//!
//! Model-body placement is already covered by `examples/test_workspace/models/pipe_orders.sql`
//! and the `example_diagnostics` / `example_workspaces` gates; this file fills in the
//! parenthesised-subquery and CTE-body positions.

#[allow(dead_code)]
mod prop_helpers;

use std::collections::{HashMap, HashSet};

use prop_helpers::duckdb_oracle::DuckDbOracle;
use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::parse;

/// Lower arbitrary SQL (pipe or standard) to a backend-printable form via the DuckDB printer.
fn lower_to_duckdb(sql: &str) -> String {
    let parsed = parse(sql);
    let dialect = SqlDialect::DuckDB;
    let caps = BackendCapabilities::duckdb();
    let ctx = PrintContext {
        dialect: &dialect,
        capabilities: &caps,
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

// ── Test 1: pipe_as_subquery_body ────────────────────────────────────────────

/// A pipe query used as a parenthesised subquery inside another FROM clause must
/// parse, lower (no `|>` in output), and produce the same result set on DuckDB as
/// its hand-written standard SQL equivalent.
///
/// Spec §"Where a pipe query may appear": "As a subquery or CTE body — anywhere a
/// parenthesised query or a WITH CTE body is legal, e.g. `FROM (FROM t |> WHERE p)`."
#[test]
fn pipe_as_subquery_body() {
    let oracle = DuckDbOracle::new();

    oracle
        .execute_ddl(
            "CREATE TABLE events (user_id INTEGER, ts INTEGER, kind VARCHAR);
             INSERT INTO events VALUES
               (1, 100, 'click'), (2, 50, 'view'), (3, 200, 'click'),
               (1, 150, 'view'), (2, 90, 'click');",
        )
        .expect("DDL setup failed");

    // Outer query uses a pipe subquery to pre-filter, then selects from it.
    let pipe_sql =
        "SELECT user_id, kind FROM (FROM events |> WHERE kind = 'click') filtered ORDER BY user_id";

    let lowered = lower_to_duckdb(pipe_sql);

    // No |> must survive into the lowered form.
    assert!(
        !lowered.contains("|>"),
        "lowered SQL must not contain |>, got:\n  {lowered}"
    );

    // The equivalent hand-written SQL.
    let standard_sql = "SELECT user_id, kind FROM (SELECT * FROM events WHERE kind = 'click') filtered ORDER BY user_id";

    // Both must execute and produce identical result sets.
    let pipe_rows = oracle.execute_query(&lowered).unwrap_or_else(|e| {
        panic!("pipe subquery lowered form failed on DuckDB:\n  {lowered}\n  error: {e}")
    });
    let std_rows = oracle.execute_query(standard_sql).unwrap_or_else(|e| {
        panic!("standard SQL failed on DuckDB:\n  {standard_sql}\n  error: {e}")
    });

    assert_eq!(
        pipe_rows, std_rows,
        "pipe-as-subquery result != standard SQL result\n  pipe lowered: {lowered}\n  standard: {standard_sql}"
    );
}

// ── Test 2: pipe_as_cte_body ─────────────────────────────────────────────────

/// A pipe query used as a CTE body (`WITH recent AS (FROM events |> WHERE ts > 0)`)
/// must parse, lower (no `|>` in output), and produce the same result set as its
/// hand-written standard SQL equivalent.
///
/// Spec §"Where a pipe query may appear": "As a subquery or CTE body — anywhere a
/// parenthesised query or a WITH CTE body is legal …
/// `WITH recent AS (FROM events |> WHERE ts > …) …`."
#[test]
fn pipe_as_cte_body() {
    let oracle = DuckDbOracle::new();

    oracle
        .execute_ddl(
            "CREATE TABLE log_entries (id INTEGER, ts INTEGER, msg VARCHAR);
             INSERT INTO log_entries VALUES
               (1, 10, 'boot'), (2, 50, 'ready'), (3, 0, 'init'),
               (4, 100, 'shutdown'), (5, 0, 'noop');",
        )
        .expect("DDL setup failed");

    // CTE whose body is a pipe query.
    let pipe_sql =
        "WITH recent AS (FROM log_entries |> WHERE ts > 0) SELECT id, msg FROM recent ORDER BY id";

    let lowered = lower_to_duckdb(pipe_sql);

    // No |> must survive into the lowered form.
    assert!(
        !lowered.contains("|>"),
        "lowered SQL must not contain |>, got:\n  {lowered}"
    );

    // The equivalent hand-written SQL.
    let standard_sql =
        "WITH recent AS (SELECT * FROM log_entries WHERE ts > 0) SELECT id, msg FROM recent ORDER BY id";

    // Both must execute and produce identical result sets.
    let pipe_rows = oracle.execute_query(&lowered).unwrap_or_else(|e| {
        panic!("pipe CTE lowered form failed on DuckDB:\n  {lowered}\n  error: {e}")
    });
    let std_rows = oracle.execute_query(standard_sql).unwrap_or_else(|e| {
        panic!("standard SQL failed on DuckDB:\n  {standard_sql}\n  error: {e}")
    });

    assert_eq!(
        pipe_rows, std_rows,
        "pipe-as-CTE result != standard SQL result\n  pipe lowered: {lowered}\n  standard: {standard_sql}"
    );
}
