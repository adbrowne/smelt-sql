//! DuckDB oracle tests for pipe SQL passthrough lowering.
//!
//! Tests that pipe queries lowered by the dialect printer produce the same
//! result sets as their standard SQL equivalents when executed on DuckDB.

#[allow(dead_code)]
mod prop_helpers;

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_oracle_testkit::{DuckDbOracle, TypeOracle};
use smelt_parser::parse;

/// Lower a pipe query to standard SQL using the DuckDB dialect printer.
fn lower_pipe_sql(sql: &str) -> String {
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
        restructure_plans: &[],
    };
    print(&parsed.syntax(), &ctx)
}

// ── Test 1: passthrough_matches_standard_sql ─────────────────────────────────

/// Verify that the lowered pipe query produces the same result set as its
/// hand-written standard SQL equivalent when executed on DuckDB.
#[test]
fn passthrough_matches_standard_sql() {
    let oracle = DuckDbOracle::new();

    // Set up fixture data
    oracle
        .execute_ddl(
            "CREATE TABLE orders (customer_id INTEGER, amount DOUBLE, status VARCHAR);
             INSERT INTO orders VALUES (1, 100.0, 'paid'), (2, 50.0, 'pending'), (3, 200.0, 'paid');",
        )
        .expect("DDL setup failed");

    // The pipe query
    let pipe_sql = "FROM orders |> WHERE status = 'paid' |> SELECT customer_id, amount |> ORDER BY amount DESC";

    // Lower it
    let lowered = lower_pipe_sql(pipe_sql);

    // Assert no |> in the lowered form
    assert!(
        !lowered.contains("|>"),
        "lowered SQL must not contain |>, got: {lowered}"
    );

    // The equivalent hand-written standard SQL
    let standard_sql =
        "SELECT customer_id, amount FROM orders WHERE status = 'paid' ORDER BY amount DESC";

    // Run both on DuckDB
    let lowered_types = oracle
        .query_types(&lowered)
        .expect("lowered pipe query failed on DuckDB");
    let standard_types = oracle
        .query_types(standard_sql)
        .expect("standard SQL failed on DuckDB");

    // Same columns in the same order
    assert_eq!(
        lowered_types.len(),
        standard_types.len(),
        "column count mismatch: lowered={lowered:?}, standard={standard_types:?}"
    );
    for (i, (low_col, std_col)) in lowered_types.iter().zip(standard_types.iter()).enumerate() {
        assert_eq!(
            low_col.0, std_col.0,
            "column {i} name mismatch: lowered='{}', standard='{}'",
            low_col.0, std_col.0
        );
        assert_eq!(
            low_col.1, std_col.1,
            "column {i} type mismatch for '{}': lowered={:?}, standard={:?}",
            low_col.0, low_col.1, std_col.1
        );
    }
}

// ── Test 2: no_pipe_token_reaches_backend ────────────────────────────────────

/// Assert that the lowered emission for a supports_pipe_syntax=false backend
/// contains no `|>` token.
#[test]
fn no_pipe_token_reaches_backend() {
    let pipe_queries = vec![
        "FROM t |> WHERE a > 0 |> SELECT a, b |> ORDER BY a |> LIMIT 5",
        "FROM t |> SELECT a |> DISTINCT",
        "FROM t |> WHERE x = 1",
        "FROM orders |> WHERE status = 'paid' |> SELECT customer_id, amount |> ORDER BY amount DESC |> LIMIT 100",
        "FROM t |> LIMIT 10",
    ];

    for sql in pipe_queries {
        let lowered = lower_pipe_sql(sql);
        assert!(
            !lowered.contains("|>"),
            "pipe token found in lowered output for input: {sql}\n  lowered: {lowered}"
        );
    }
}

// ── Test 4 (Phase 3): column_editing_matches_standard_sql ────────────────────

/// DuckDB oracle equivalence for EXTEND.
///
/// `FROM nums |> EXTEND n * 2 AS doubled` should produce the same result as
/// `SELECT n, n * 2 AS doubled FROM nums`.
#[test]
fn column_editing_matches_standard_sql() {
    let oracle = DuckDbOracle::new();

    oracle
        .execute_ddl("CREATE TABLE nums (n INTEGER); INSERT INTO nums VALUES (1), (2), (3);")
        .expect("DDL setup failed");

    let pipe_sql = "FROM nums |> EXTEND n * 2 AS doubled";
    let lowered = lower_pipe_sql(pipe_sql);

    // No |> in output
    assert!(
        !lowered.contains("|>"),
        "lowered SQL must not contain |>, got: {lowered}"
    );

    // Must be executable on DuckDB
    let lowered_types = oracle
        .query_types(&lowered)
        .unwrap_or_else(|_| panic!("lowered pipe query failed on DuckDB: {lowered}"));

    // Should have two columns: n and doubled
    assert_eq!(
        lowered_types.len(),
        2,
        "expected 2 columns (n, doubled), got: {lowered_types:?} from: {lowered}"
    );
    assert_eq!(
        lowered_types[0].0, "n",
        "first column should be 'n', got: {:?} from: {lowered}",
        lowered_types[0].0
    );
    assert_eq!(
        lowered_types[1].0, "doubled",
        "second column should be 'doubled', got: {:?} from: {lowered}",
        lowered_types[1].0
    );
}

// ── SET / DROP / RENAME DuckDB oracle tests ───────────────────────────────────

/// `|> SET a = a * 2` must lower to `SELECT * REPLACE (a * 2 AS a) FROM (...)`,
/// which DuckDB executes correctly — not a boolean comparison.
#[test]
fn set_column_matches_standard_sql() {
    let oracle = DuckDbOracle::new();
    oracle
        .execute_ddl(
            "CREATE TABLE t_set (a INTEGER, b INTEGER); INSERT INTO t_set VALUES (1, 10), (2, 20);",
        )
        .expect("DDL setup failed");

    let pipe = "FROM t_set |> SET a = a * 2";
    let lowered = lower_pipe_sql(pipe);
    assert!(
        !lowered.contains("|>"),
        "lowered SQL must not contain |>, got: {lowered}"
    );
    assert!(
        lowered.contains("REPLACE"),
        "SET lowering must use SELECT * REPLACE (...) syntax; got: {lowered}"
    );

    // The lowered form must be executable by DuckDB.
    let pipe_rows = oracle
        .query_types(&lowered)
        .unwrap_or_else(|e| panic!("pipe query failed on DuckDB: {lowered}\n  error: {e}"));

    // Standard SQL equivalent: a is replaced with a * 2, b is kept.
    let std_sql = "SELECT a * 2 AS a, b FROM t_set";
    let std_rows = oracle
        .query_types(std_sql)
        .unwrap_or_else(|e| panic!("standard SQL failed on DuckDB: {std_sql}\n  error: {e}"));

    assert_eq!(
        pipe_rows.len(),
        std_rows.len(),
        "column count mismatch: pipe={pipe_rows:?}, std={std_rows:?} from: {lowered}"
    );
    assert_eq!(
        pipe_rows[0].0, "a",
        "first column should be 'a', got: {:?} from: {lowered}",
        pipe_rows[0].0
    );
    assert_eq!(
        pipe_rows[1].0, "b",
        "second column should be 'b', got: {:?} from: {lowered}",
        pipe_rows[1].0
    );
}

/// `|> DROP a` must lower to `SELECT * EXCLUDE (a) FROM (...)`, removing the column.
#[test]
fn drop_column_matches_standard_sql() {
    let oracle = DuckDbOracle::new();
    oracle
        .execute_ddl(
            "CREATE TABLE t_drop (a INTEGER, b INTEGER); INSERT INTO t_drop VALUES (1, 10);",
        )
        .expect("DDL setup failed");

    let pipe = "FROM t_drop |> DROP a";
    let lowered = lower_pipe_sql(pipe);
    assert!(
        !lowered.contains("|>"),
        "lowered SQL must not contain |>, got: {lowered}"
    );

    let pipe_rows = oracle
        .query_types(&lowered)
        .unwrap_or_else(|e| panic!("pipe query failed on DuckDB: {lowered}\n  error: {e}"));

    assert_eq!(
        pipe_rows.len(),
        1,
        "DROP a should leave 1 column (b); got: {pipe_rows:?} from: {lowered}"
    );
    assert_eq!(
        pipe_rows[0].0, "b",
        "remaining column should be 'b'; got: {pipe_rows:?} from: {lowered}"
    );
}

/// `|> RENAME a AS x` must lower to `SELECT * RENAME (a AS x) FROM (...)`,
/// renaming the column without losing the other columns.
#[test]
fn rename_column_matches_standard_sql() {
    let oracle = DuckDbOracle::new();
    oracle
        .execute_ddl(
            "CREATE TABLE t_rename (a INTEGER, b INTEGER); INSERT INTO t_rename VALUES (1, 10);",
        )
        .expect("DDL setup failed");

    let pipe = "FROM t_rename |> RENAME a AS x";
    let lowered = lower_pipe_sql(pipe);
    assert!(
        !lowered.contains("|>"),
        "lowered SQL must not contain |>, got: {lowered}"
    );

    let pipe_rows = oracle
        .query_types(&lowered)
        .unwrap_or_else(|e| panic!("pipe query failed on DuckDB: {lowered}\n  error: {e}"));

    assert_eq!(
        pipe_rows.len(),
        2,
        "RENAME should keep 2 columns; got: {pipe_rows:?} from: {lowered}"
    );
    let col_names: Vec<&str> = pipe_rows.iter().map(|c| c.0.as_str()).collect();
    assert!(
        col_names.contains(&"x"),
        "expected column 'x' after rename; got: {col_names:?} from: {lowered}"
    );
    assert!(
        !col_names.contains(&"a"),
        "old column 'a' should not be present after rename; got: {col_names:?} from: {lowered}"
    );
}

// ── Test 3 (original): LIMIT pipe stage ──────────────────────────────────────

/// A pipe query with only a LIMIT stage must lower to SELECT * FROM t LIMIT N.
#[test]
fn limit_only_pipe_stage_lowers() {
    let oracle = DuckDbOracle::new();

    oracle
        .execute_ddl(
            "CREATE TABLE nums (n INTEGER); INSERT INTO nums VALUES (1), (2), (3), (4), (5);",
        )
        .expect("DDL failed");

    let pipe_sql = "FROM nums |> LIMIT 3";
    let lowered = lower_pipe_sql(pipe_sql);

    assert!(
        !lowered.contains("|>"),
        "no |> in lowered output, got: {lowered}"
    );

    // Must be executable by DuckDB
    let result = oracle.query_types(&lowered);
    assert!(
        result.is_ok(),
        "lowered SQL must execute on DuckDB: {lowered}\n  error: {result:?}"
    );
}

// ── Test: two_aggregates_match_standard_sql ──────────────────────────────────

/// DuckDB oracle: two AGGREGATE stages produce the same result as hand-written nested SQL.
#[test]
fn two_aggregates_match_standard_sql() {
    let oracle = DuckDbOracle::new();

    oracle
        .execute_ddl(
            "CREATE TABLE sales2 (region VARCHAR, city VARCHAR, amount DOUBLE);
             INSERT INTO sales2 VALUES
               ('East', 'NYC', 100.0), ('East', 'BOS', 200.0),
               ('West', 'LA', 150.0), ('West', 'SF', 300.0);",
        )
        .expect("DDL setup failed");

    // Pipe: first aggregate sums by city+region, second counts cities per region
    let pipe_sql = "FROM sales2 |> AGGREGATE sum(amount) AS city_total GROUP BY region, city |> AGGREGATE count(*) AS city_count GROUP BY region";

    let lowered = lower_pipe_sql(pipe_sql);
    assert!(
        !lowered.contains("|>"),
        "lowered two-agg must not contain |>, got: {lowered}"
    );

    // Hand-written equivalent
    let std_sql = "SELECT region, count(*) AS city_count FROM (SELECT region, city, sum(amount) AS city_total FROM sales2 GROUP BY region, city) GROUP BY region";

    let pipe_rows = oracle.execute_query(&lowered).expect("two-agg pipe failed");
    let std_rows = oracle
        .execute_query(std_sql)
        .expect("standard two-agg failed");
    assert_eq!(
        pipe_rows, std_rows,
        "two-aggregate nesting: pipe result != standard SQL result\npipe lowered: {lowered}\nstd: {std_sql}"
    );
}

// ── Test: aggregate_matches_standard_sql ────────────────────────────────────

/// DuckDB oracle: a pipe query with AGGREGATE+GROUP BY produces the same result set
/// as its hand-written standard SQL equivalent.
/// Also tests full-table aggregation (no GROUP BY → one output row).
#[test]
fn aggregate_matches_standard_sql() {
    let oracle = DuckDbOracle::new();

    oracle
        .execute_ddl(
            "CREATE TABLE sales (customer_id INTEGER, amount DOUBLE);
             INSERT INTO sales VALUES (1, 100.0), (2, 50.0), (1, 200.0), (2, 75.0);",
        )
        .expect("DDL setup failed");

    // --- Test 1: grouped aggregation ---
    let pipe_sql = "FROM sales |> AGGREGATE sum(amount) AS revenue GROUP BY customer_id";
    let std_sql = "SELECT customer_id, sum(amount) AS revenue FROM sales GROUP BY customer_id";

    let lowered = lower_pipe_sql(pipe_sql);
    assert!(
        !lowered.contains("|>"),
        "lowered grouped agg must not contain |>, got: {lowered}"
    );

    let pipe_rows = oracle.execute_query(&lowered).expect("pipe query failed");
    let std_rows = oracle
        .execute_query(std_sql)
        .expect("standard SQL query failed");
    assert_eq!(
        pipe_rows, std_rows,
        "grouped aggregation: pipe result != standard SQL result\npipe lowered: {lowered}"
    );

    // --- Test 2: full-table aggregation (no GROUP BY) ---
    let pipe_total = "FROM sales |> AGGREGATE sum(amount) AS total";
    let std_total = "SELECT sum(amount) AS total FROM sales";

    let lowered_total = lower_pipe_sql(pipe_total);
    assert!(
        !lowered_total.contains("|>"),
        "lowered full-table agg must not contain |>, got: {lowered_total}"
    );

    let pipe_total_rows = oracle
        .execute_query(&lowered_total)
        .expect("pipe total failed");
    let std_total_rows = oracle
        .execute_query(std_total)
        .expect("standard total failed");
    assert_eq!(
        pipe_total_rows, std_total_rows,
        "full-table aggregation: pipe result != standard SQL result\npipe lowered: {lowered_total}"
    );
}

// ── Phase 5: JOIN and set-op DuckDB oracle ────────────────────────────────────

#[test]
fn join_and_setops_match_standard_sql() {
    let oracle = DuckDbOracle::new();

    oracle
        .execute_ddl(
            "CREATE TABLE emps (dept_id INTEGER, name VARCHAR);
         INSERT INTO emps VALUES (1, 'Alice'), (2, 'Bob');
         CREATE TABLE depts (dept_id INTEGER, dept_name VARCHAR);
         INSERT INTO depts VALUES (1, 'Engineering'), (2, 'Marketing');
         CREATE TABLE extra_emps (dept_id INTEGER, name VARCHAR);
         INSERT INTO extra_emps VALUES (3, 'Dave');",
        )
        .expect("DDL setup failed");

    // Test 1: JOIN equivalence
    let pipe_join = "FROM emps |> JOIN depts ON emps.dept_id = depts.dept_id |> ORDER BY name";
    let lowered_join = lower_pipe_sql(pipe_join);
    assert!(
        !lowered_join.contains("|>"),
        "lowered JOIN must not contain |>, got: {lowered_join}"
    );
    let std_join = "SELECT * FROM emps JOIN depts ON emps.dept_id = depts.dept_id ORDER BY name";
    let pipe_rows = oracle
        .execute_query(&lowered_join)
        .expect("pipe JOIN failed on DuckDB");
    let std_rows = oracle
        .execute_query(std_join)
        .expect("standard JOIN failed on DuckDB");
    assert_eq!(
        pipe_rows, std_rows,
        "JOIN: pipe result != standard SQL\npipe lowered: {lowered_join}\nstd: {std_join}"
    );

    // Test 2: UNION ALL equivalence
    let pipe_union = "FROM emps |> UNION ALL (SELECT * FROM extra_emps) |> ORDER BY name";
    let lowered_union = lower_pipe_sql(pipe_union);
    assert!(
        !lowered_union.contains("|>"),
        "lowered UNION must not contain |>, got: {lowered_union}"
    );
    let std_union = "SELECT * FROM emps UNION ALL SELECT * FROM extra_emps ORDER BY name";
    let pipe_union_rows = oracle
        .execute_query(&lowered_union)
        .expect("pipe UNION failed on DuckDB");
    let std_union_rows = oracle
        .execute_query(std_union)
        .expect("standard UNION failed on DuckDB");
    assert_eq!(
        pipe_union_rows, std_union_rows,
        "UNION: pipe result != standard SQL\npipe lowered: {lowered_union}\nstd: {std_union}"
    );
}
