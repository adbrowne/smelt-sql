//! Tests for known parser gaps
//!
//! This file contains explicit tests for each known parser gap between smelt and PostgreSQL.
//! These tests serve as documentation and regression tests.
//!
//! Run with: cargo test -p smelt-parser-compat --test known_gaps

use smelt_parser_compat::{gaps, PgParseResult, SmeltParseResult};

// ===== Helper Functions =====

/// Test that smelt fails but pg_query succeeds
fn assert_smelt_fails_pg_succeeds(sql: &str, gap_id: &str) {
    let smelt = SmeltParseResult::parse(sql);
    let pg = PgParseResult::parse(sql);

    assert!(
        !smelt.success,
        "{}: Expected smelt to fail, but it succeeded\nSQL: {}",
        gap_id, sql
    );
    assert!(
        pg.success,
        "{}: Expected pg_query to succeed, but it failed: {:?}\nSQL: {}",
        gap_id, pg.error, sql
    );
}

/// Test that pg_query fails but smelt succeeds (smelt extension)
fn assert_pg_fails_smelt_succeeds(sql: &str, gap_id: &str) {
    let smelt = SmeltParseResult::parse(sql);
    let pg = PgParseResult::parse(sql);

    assert!(
        smelt.success,
        "{}: Expected smelt to succeed, but it failed: {:?}\nSQL: {}",
        gap_id, smelt.errors, sql
    );
    assert!(
        !pg.success,
        "{}: Expected pg_query to fail, but it succeeded\nSQL: {}",
        gap_id, sql
    );
}

/// Test that both parsers succeed
fn assert_both_succeed(sql: &str, description: &str) {
    let smelt = SmeltParseResult::parse(sql);
    let pg = PgParseResult::parse(sql);

    assert!(
        smelt.success,
        "{}: Expected smelt to succeed, but it failed: {:?}\nSQL: {}",
        description, smelt.errors, sql
    );
    assert!(
        pg.success,
        "{}: Expected pg_query to succeed, but it failed: {:?}\nSQL: {}",
        description, pg.error, sql
    );
}

// ===== Gap Summary Test =====

#[test]
fn test_gap_summary() {
    let summary = gaps::GapSummary::compute();

    println!("Known Parser Gaps Summary:");
    println!("  Total gaps: {}", summary.total);
    println!("  smelt fails, pg succeeds: {}", summary.smelt_fails);
    println!("  pg fails, smelt succeeds: {}", summary.pg_fails);
    println!("  Fingerprint mismatches: {}", summary.fingerprint_mismatch);
    println!();
    println!("By severity:");
    println!("  High: {}", summary.high_severity);
    println!("  Medium: {}", summary.medium_severity);
    println!("  Low: {}", summary.low_severity);
    println!();
    println!("Planned fixes: {}", summary.planned_fix);

    assert!(summary.total > 0, "Should have documented gaps");
}

// ===== Tests for smelt_fails gaps (PostgreSQL features not in smelt) =====

#[test]
fn test_gap_array_subscript() {
    // Array subscript notation
    assert_smelt_fails_pg_succeeds("SELECT arr[1] FROM t", "array_subscript");
    assert_smelt_fails_pg_succeeds("SELECT arr[1:3] FROM t", "array_subscript");
    assert_smelt_fails_pg_succeeds("SELECT matrix[1][2] FROM t", "array_subscript");
}

#[test]
fn test_gap_json_operators() {
    // JSON operators
    assert_smelt_fails_pg_succeeds("SELECT data->>'name' FROM t", "json_operators");
    assert_smelt_fails_pg_succeeds("SELECT data->'nested' FROM t", "json_operators");
    assert_smelt_fails_pg_succeeds("SELECT data#>'{a,b}' FROM t", "json_operators");
    assert_smelt_fails_pg_succeeds("SELECT data#>>'{a,b}' FROM t", "json_operators");
}

#[test]
fn test_gap_pattern_match_operators() {
    // Pattern matching operators
    assert_smelt_fails_pg_succeeds(
        "SELECT * FROM t WHERE name ~ '^test'",
        "pattern_match_operators",
    );
    assert_smelt_fails_pg_succeeds(
        "SELECT * FROM t WHERE name ~* '^test'",
        "pattern_match_operators",
    );
    assert_smelt_fails_pg_succeeds(
        "SELECT * FROM t WHERE name !~ '^test'",
        "pattern_match_operators",
    );
}

#[test]
fn test_gap_string_concat_operator() {
    // String concatenation operator
    assert_smelt_fails_pg_succeeds("SELECT 'a' || 'b'", "string_concat_operator");
    assert_smelt_fails_pg_succeeds(
        "SELECT first_name || ' ' || last_name FROM users",
        "string_concat_operator",
    );
}

#[test]
fn test_gap_any_all_some() {
    // ANY/ALL/SOME array comparisons
    assert_smelt_fails_pg_succeeds(
        "SELECT * FROM t WHERE id = ANY(ARRAY[1,2,3])",
        "any_all_some",
    );
    assert_smelt_fails_pg_succeeds(
        "SELECT * FROM t WHERE id = ALL(ARRAY[1,2,3])",
        "any_all_some",
    );
}

// ===== Tests for pg_fails gaps (smelt extensions not in PostgreSQL) =====

#[test]
fn test_gap_trailing_comma() {
    // Trailing commas (DuckDB extension) - smelt accepts, pg_query rejects
    assert_pg_fails_smelt_succeeds("SELECT a, b, c, FROM t", "trailing_comma");
}

// ===== Tests for features that DO work in both parsers =====
// These demonstrate the compatibility between smelt and PostgreSQL

#[test]
fn test_both_work_basic_select() {
    assert_both_succeed("SELECT * FROM users", "basic select");
    assert_both_succeed("SELECT id, name FROM users", "column list");
    assert_both_succeed("SELECT id AS user_id FROM users", "column alias");
}

#[test]
fn test_both_work_where() {
    assert_both_succeed("SELECT * FROM users WHERE id = 1", "simple where");
    assert_both_succeed(
        "SELECT * FROM users WHERE id = 1 AND name = 'test'",
        "where with AND",
    );
    assert_both_succeed(
        "SELECT * FROM users WHERE id = 1 OR name = 'test'",
        "where with OR",
    );
}

#[test]
fn test_both_work_joins() {
    assert_both_succeed("SELECT * FROM a JOIN b ON a.id = b.id", "simple JOIN");
    assert_both_succeed("SELECT * FROM a INNER JOIN b ON a.id = b.id", "INNER JOIN");
    assert_both_succeed("SELECT * FROM a LEFT JOIN b ON a.id = b.id", "LEFT JOIN");
    assert_both_succeed("SELECT * FROM a RIGHT JOIN b ON a.id = b.id", "RIGHT JOIN");
    assert_both_succeed("SELECT * FROM a FULL JOIN b ON a.id = b.id", "FULL JOIN");
    assert_both_succeed("SELECT * FROM a CROSS JOIN b", "CROSS JOIN");
    assert_both_succeed("SELECT * FROM a JOIN b USING (id)", "JOIN with USING");
}

#[test]
fn test_both_work_group_by() {
    assert_both_succeed("SELECT city, COUNT(*) FROM users GROUP BY city", "GROUP BY");
    assert_both_succeed(
        "SELECT city, COUNT(*) FROM users GROUP BY city HAVING COUNT(*) > 5",
        "GROUP BY with HAVING",
    );
}

#[test]
fn test_both_work_order_by() {
    assert_both_succeed("SELECT * FROM users ORDER BY name", "ORDER BY");
    assert_both_succeed("SELECT * FROM users ORDER BY name ASC", "ORDER BY ASC");
    assert_both_succeed("SELECT * FROM users ORDER BY name DESC", "ORDER BY DESC");
    assert_both_succeed(
        "SELECT * FROM users ORDER BY name NULLS FIRST",
        "ORDER BY NULLS FIRST",
    );
    assert_both_succeed(
        "SELECT * FROM users ORDER BY name NULLS LAST",
        "ORDER BY NULLS LAST",
    );
}

#[test]
fn test_both_work_limit() {
    assert_both_succeed("SELECT * FROM users LIMIT 10", "LIMIT");
    assert_both_succeed("SELECT * FROM users LIMIT 10 OFFSET 5", "LIMIT OFFSET");
    assert_both_succeed("SELECT * FROM users LIMIT ALL", "LIMIT ALL");
}

#[test]
fn test_both_work_distinct() {
    assert_both_succeed("SELECT DISTINCT city FROM users", "DISTINCT");
    assert_both_succeed("SELECT COUNT(DISTINCT city) FROM users", "COUNT DISTINCT");
}

#[test]
fn test_both_work_cte() {
    assert_both_succeed(
        "WITH t AS (SELECT * FROM users) SELECT * FROM t",
        "simple CTE",
    );
    assert_both_succeed(
        "WITH t1 AS (SELECT * FROM a), t2 AS (SELECT * FROM b) SELECT * FROM t1 JOIN t2 ON t1.id = t2.id",
        "multiple CTEs",
    );
}

#[test]
fn test_both_work_window_functions() {
    assert_both_succeed("SELECT ROW_NUMBER() OVER () FROM users", "ROW_NUMBER");
    assert_both_succeed(
        "SELECT ROW_NUMBER() OVER (ORDER BY id) FROM users",
        "ROW_NUMBER with ORDER BY",
    );
    assert_both_succeed(
        "SELECT SUM(amount) OVER (PARTITION BY user_id) FROM orders",
        "SUM with PARTITION BY",
    );
    assert_both_succeed(
        "SELECT SUM(amount) OVER (ORDER BY date ROWS UNBOUNDED PRECEDING) FROM orders",
        "window frame",
    );
}

#[test]
fn test_both_work_case() {
    assert_both_succeed(
        "SELECT CASE WHEN x > 0 THEN 'positive' ELSE 'negative' END FROM t",
        "searched CASE",
    );
    assert_both_succeed(
        "SELECT CASE x WHEN 1 THEN 'one' WHEN 2 THEN 'two' ELSE 'other' END FROM t",
        "simple CASE",
    );
}

#[test]
fn test_both_work_cast() {
    assert_both_succeed("SELECT CAST(x AS INTEGER) FROM t", "CAST");
    assert_both_succeed("SELECT CAST(x AS VARCHAR(100)) FROM t", "CAST with params");
    assert_both_succeed("SELECT x::INTEGER FROM t", "PostgreSQL cast");
}

#[test]
fn test_both_work_subqueries() {
    assert_both_succeed("SELECT * FROM (SELECT id FROM users) t", "subquery in FROM");
    assert_both_succeed(
        "SELECT * FROM users WHERE id IN (SELECT user_id FROM orders)",
        "IN subquery",
    );
    assert_both_succeed(
        "SELECT * FROM users WHERE EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id)",
        "EXISTS",
    );
}

#[test]
fn test_both_work_between_in() {
    assert_both_succeed("SELECT * FROM t WHERE x BETWEEN 1 AND 10", "BETWEEN");
    assert_both_succeed("SELECT * FROM t WHERE x IN (1, 2, 3)", "IN list");
}

#[test]
fn test_both_work_union() {
    assert_both_succeed("SELECT id FROM a UNION SELECT id FROM b", "UNION");
    assert_both_succeed("SELECT id FROM a UNION ALL SELECT id FROM b", "UNION ALL");
}

#[test]
fn test_both_work_is_null() {
    assert_both_succeed("SELECT * FROM t WHERE x IS NULL", "IS NULL");
    assert_both_succeed("SELECT * FROM t WHERE x IS NOT NULL", "IS NOT NULL");
}

// ===== Tests verifying smelt extensions work in smelt but are also valid pg syntax =====

#[test]
fn test_smelt_extensions_valid_pg_syntax() {
    // These tests verify that smelt extensions happen to be valid PostgreSQL syntax
    // (function calls with schema prefix and named parameters)

    // smelt.ref() is valid PostgreSQL function call syntax
    assert_both_succeed(
        "SELECT * FROM smelt.ref('users')",
        "smelt.ref() is valid pg function syntax",
    );

    // smelt.source() is valid PostgreSQL function call syntax
    assert_both_succeed(
        "SELECT * FROM smelt.source('raw.users')",
        "smelt.source() is valid pg function syntax",
    );

    // Named parameters with => are valid PostgreSQL function syntax
    assert_both_succeed(
        "SELECT * FROM func(arg => 'value')",
        "named parameters are valid pg syntax",
    );
}

// ===== Gap pattern matching tests =====

#[test]
fn test_gap_pattern_detection() {
    // Verify the gap detection patterns work correctly for actual gaps
    assert!(gaps::is_known_gap("SELECT arr[1] FROM t", "smelt_fails"));
    assert!(gaps::is_known_gap(
        "SELECT data->>'name' FROM t",
        "smelt_fails"
    ));
    assert!(gaps::is_known_gap("SELECT 'a' || 'b'", "smelt_fails"));
    assert!(gaps::is_known_gap("SELECT a, b, FROM t", "pg_fails"));
}

#[test]
fn test_get_matching_gaps() {
    let gaps = gaps::get_matching_gaps("SELECT arr[1], data->>'name' FROM t");
    assert!(gaps.contains(&"array_subscript"));
    assert!(gaps.contains(&"json_operators"));
}

// ===== Tests for features smelt supports that were initially thought to be gaps =====
// These document features that smelt actually implements

#[test]
fn test_smelt_supports_like() {
    // LIKE is supported by smelt
    assert_both_succeed("SELECT * FROM t WHERE name LIKE '%test%'", "LIKE operator");
    assert_both_succeed(
        "SELECT * FROM t WHERE name ILIKE '%TEST%'",
        "ILIKE operator",
    );
}

#[test]
fn test_smelt_supports_intersect_except() {
    // INTERSECT and EXCEPT are supported by smelt
    assert_both_succeed("SELECT id FROM a INTERSECT SELECT id FROM b", "INTERSECT");
    assert_both_succeed("SELECT id FROM a EXCEPT SELECT id FROM b", "EXCEPT");
}

#[test]
fn test_smelt_supports_grouping_sets() {
    // GROUPING SETS, CUBE, ROLLUP are supported by smelt
    assert_both_succeed(
        "SELECT a, b, SUM(c) FROM t GROUP BY GROUPING SETS ((a), (b), ())",
        "GROUPING SETS",
    );
    assert_both_succeed("SELECT a, b, SUM(c) FROM t GROUP BY CUBE (a, b)", "CUBE");
    assert_both_succeed(
        "SELECT a, b, SUM(c) FROM t GROUP BY ROLLUP (a, b)",
        "ROLLUP",
    );
}

#[test]
fn test_gap_row_constructor() {
    // ROW constructor is NOT supported by smelt
    assert_smelt_fails_pg_succeeds("SELECT ROW(1, 2, 3)", "row_constructor");
}

#[test]
fn test_gap_array_literal() {
    // ARRAY literal syntax is NOT supported by smelt
    assert_smelt_fails_pg_succeeds("SELECT ARRAY[1, 2, 3]", "array_literal");
}

#[test]
fn test_gap_values_clause() {
    // VALUES clause is NOT supported by smelt
    assert_smelt_fails_pg_succeeds("VALUES (1, 'a'), (2, 'b')", "values_clause");
}

#[test]
fn test_smelt_supports_natural_join() {
    // NATURAL JOIN is supported by smelt
    assert_both_succeed("SELECT * FROM a NATURAL JOIN b", "NATURAL JOIN");
}

#[test]
fn test_smelt_supports_interval() {
    // Interval literals are supported by smelt
    assert_both_succeed("SELECT INTERVAL '1 day'", "INTERVAL literal");
}

#[test]
fn test_smelt_supports_type_qualified_literal() {
    // Type-qualified literals are supported by smelt
    assert_both_succeed("SELECT DATE '2024-01-01'", "DATE literal");
    assert_both_succeed(
        "SELECT TIMESTAMP '2024-01-01 12:00:00'",
        "TIMESTAMP literal",
    );
}

#[test]
fn test_smelt_supports_at_time_zone() {
    // AT TIME ZONE is supported by smelt
    assert_both_succeed(
        "SELECT created_at AT TIME ZONE 'UTC' FROM t",
        "AT TIME ZONE",
    );
}

#[test]
fn test_smelt_supports_fetch_clause() {
    // FETCH FIRST/NEXT is supported by smelt
    assert_both_succeed("SELECT * FROM t FETCH FIRST 10 ROWS ONLY", "FETCH FIRST");
}

#[test]
fn test_smelt_supports_for_update() {
    // FOR UPDATE is supported by smelt
    assert_both_succeed("SELECT * FROM t FOR UPDATE", "FOR UPDATE");
}
