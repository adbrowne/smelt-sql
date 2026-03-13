//! Parse equivalence property tests
//!
//! These tests verify that smelt-parser produces semantically equivalent results
//! to PostgreSQL's parser (via pg_query).
//!
//! Run with: cargo test -p smelt-parser-compat --test parse_equivalence

use proptest::prelude::*;
use smelt_parser_compat::{
    compare_all_parse_results, compare_parse_results, compare_spark_parse_results, gaps,
    generators, PgParseResult, SmeltParseResult, SparkSqlparserResult,
};

/// Configuration for property tests
const PROPTEST_CASES: u32 = 500;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(PROPTEST_CASES))]

    /// Property: If smelt parses without errors, pg_query should too (for standard SQL)
    ///
    /// Exceptions:
    /// - smelt extensions (smelt.ref, smelt.source, =>)
    /// - DuckDB-friendly syntax (trailing commas)
    #[test]
    fn prop_smelt_valid_implies_pg_valid(sql in generators::simple_select()) {
        let smelt_result = SmeltParseResult::parse(&sql);
        if smelt_result.success {
            // Skip smelt-specific extensions
            if smelt_parser_compat::has_smelt_extensions(&sql) {
                return Ok(());
            }

            let pg_result = PgParseResult::parse(&sql);

            // If smelt succeeds, pg_query should too for standard SQL
            // (unless it's a known intentional divergence)
            if !pg_result.success && !gaps::is_known_gap(&sql, "pg_fails") {
                prop_assert!(false,
                    "smelt succeeded but pg_query failed\n\
                     SQL: {}\n\
                     pg_query error: {:?}",
                    sql, pg_result.error
                );
            }
        }
    }

    /// Property: If pg_query parses, smelt should too (unless it's a known gap)
    #[test]
    fn prop_pg_valid_implies_smelt_valid(sql in generators::simple_select()) {
        let pg_result = PgParseResult::parse(&sql);
        if pg_result.success {
            let smelt_result = SmeltParseResult::parse(&sql);

            if !smelt_result.success && !gaps::is_known_gap(&sql, "smelt_fails") {
                prop_assert!(false,
                    "pg_query succeeded but smelt failed\n\
                     SQL: {}\n\
                     smelt errors: {:?}",
                    sql, smelt_result.errors
                );
            }
        }
    }

    /// Property: Round-trip semantic equivalence
    ///
    /// If smelt parses SQL successfully, prints it back, and pg_query parses both,
    /// the fingerprints should match.
    #[test]
    fn prop_semantic_equivalence(sql in generators::simple_select()) {
        if smelt_parser_compat::has_smelt_extensions(&sql) {
            return Ok(());
        }

        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: GROUP BY queries maintain equivalence
    #[test]
    fn prop_group_by_equivalence(sql in generators::select_with_group_by()) {
        if smelt_parser_compat::has_smelt_extensions(&sql) {
            return Ok(());
        }

        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: JOIN queries maintain equivalence
    #[test]
    fn prop_join_equivalence(sql in generators::select_with_join()) {
        if smelt_parser_compat::has_smelt_extensions(&sql) {
            return Ok(());
        }

        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: CTE queries maintain equivalence
    #[test]
    fn prop_cte_equivalence(sql in generators::select_with_cte()) {
        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: Window function queries maintain equivalence
    #[test]
    fn prop_window_function_equivalence(sql in generators::select_with_window()) {
        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: UNION queries maintain equivalence
    #[test]
    fn prop_union_equivalence(sql in generators::select_union()) {
        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: CASE expressions maintain equivalence
    #[test]
    fn prop_case_equivalence(sql in generators::select_with_case()) {
        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: IN subquery queries maintain equivalence
    #[test]
    fn prop_in_subquery_equivalence(sql in generators::select_with_in_subquery()) {
        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: EXISTS queries maintain equivalence
    #[test]
    fn prop_exists_equivalence(sql in generators::select_with_exists()) {
        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: BETWEEN queries maintain equivalence
    #[test]
    fn prop_between_equivalence(sql in generators::select_with_between()) {
        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: CAST queries maintain equivalence
    #[test]
    fn prop_cast_equivalence(sql in generators::select_with_cast()) {
        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Property: DISTINCT queries maintain equivalence
    #[test]
    fn prop_distinct_equivalence(sql in generators::select_distinct()) {
        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// Test with all query types
    #[test]
    fn prop_any_select_equivalence(sql in generators::any_select()) {
        if smelt_parser_compat::has_smelt_extensions(&sql) {
            return Ok(());
        }

        match compare_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }
}

// ===== Explicit unit tests for edge cases =====

#[test]
fn test_simple_select() {
    let sql = "SELECT * FROM users";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_alias() {
    let sql = "SELECT name AS user_name FROM users";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_where() {
    let sql = "SELECT * FROM users WHERE id = 1";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_order_by() {
    let sql = "SELECT * FROM users ORDER BY name ASC";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_limit() {
    let sql = "SELECT * FROM users LIMIT 10";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_limit_offset() {
    let sql = "SELECT * FROM users LIMIT 10 OFFSET 5";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_inner_join() {
    let sql = "SELECT * FROM users INNER JOIN orders ON users.id = orders.user_id";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_left_join() {
    let sql = "SELECT * FROM users LEFT JOIN orders ON users.id = orders.user_id";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_group_by() {
    let sql = "SELECT city, COUNT(*) FROM users GROUP BY city";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_having() {
    let sql = "SELECT city, COUNT(*) FROM users GROUP BY city HAVING COUNT(*) > 5";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_distinct() {
    let sql = "SELECT DISTINCT city FROM users";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_cte() {
    let sql =
        "WITH active_users AS (SELECT * FROM users WHERE active = true) SELECT * FROM active_users";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_window_function() {
    let sql = "SELECT id, ROW_NUMBER() OVER (ORDER BY created_at) FROM users";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_case() {
    let sql = "SELECT CASE WHEN status = 'active' THEN 1 ELSE 0 END FROM users";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_cast() {
    let sql = "SELECT CAST(amount AS INTEGER) FROM orders";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_between() {
    let sql = "SELECT * FROM orders WHERE amount BETWEEN 10 AND 100";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_in() {
    let sql = "SELECT * FROM users WHERE id IN (1, 2, 3)";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_is_null() {
    let sql = "SELECT * FROM users WHERE email IS NULL";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_union() {
    let sql = "SELECT id FROM users UNION SELECT id FROM customers";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_union_all() {
    let sql = "SELECT id FROM users UNION ALL SELECT id FROM customers";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_exists() {
    let sql =
        "SELECT * FROM users WHERE EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id)";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_subquery_in_from() {
    let sql = "SELECT * FROM (SELECT id, name FROM users) AS t";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_select_with_multiple_joins() {
    let sql = "SELECT * FROM users INNER JOIN orders ON users.id = orders.user_id LEFT JOIN products ON orders.product_id = products.id";
    assert!(compare_parse_results(sql).is_ok());
}

#[test]
fn test_complex_query() {
    let sql = r#"
        WITH recent_orders AS (
            SELECT user_id, COUNT(*) as order_count
            FROM orders
            WHERE created_at > '2024-01-01'
            GROUP BY user_id
        )
        SELECT u.name, ro.order_count, ROW_NUMBER() OVER (ORDER BY ro.order_count DESC) as rank
        FROM users u
        INNER JOIN recent_orders ro ON u.id = ro.user_id
        WHERE ro.order_count > 5
        ORDER BY ro.order_count DESC
        LIMIT 100
    "#;
    assert!(compare_parse_results(sql).is_ok());
}

// ===== Spark / sqlparser-databricks property tests =====

proptest! {
    #![proptest_config(ProptestConfig::with_cases(PROPTEST_CASES))]

    /// Standard SQL generators tested against sqlparser-databricks
    #[test]
    fn prop_standard_sql_spark_valid(sql in generators::any_select()) {
        if smelt_parser_compat::has_smelt_extensions(&sql) {
            return Ok(());
        }

        match compare_spark_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    /// If smelt parses Spark-specific SQL, sqlparser-databricks should too
    #[test]
    fn prop_smelt_valid_implies_spark_valid(sql in generators::any_select()) {
        let smelt_result = SmeltParseResult::parse(&sql);
        if smelt_result.success {
            let spark_result = SparkSqlparserResult::parse(&sql);
            if !spark_result.success
                && !gaps::is_known_gap(&sql, "spark_fails")
            {
                prop_assert!(false,
                    "smelt succeeded but sqlparser-databricks failed\n\
                     SQL: {}\n\
                     sqlparser error: {:?}",
                    sql, spark_result.error
                );
            }
        }
    }

    /// If sqlparser-databricks parses, smelt should too (unless known gap)
    #[test]
    fn prop_spark_valid_implies_smelt_valid(sql in generators::any_select()) {
        let spark_result = SparkSqlparserResult::parse(&sql);
        if spark_result.success {
            let smelt_result = SmeltParseResult::parse(&sql);
            if !smelt_result.success
                && !gaps::is_known_gap(&sql, "smelt_fails")
            {
                prop_assert!(false,
                    "sqlparser-databricks succeeded but smelt failed\n\
                     SQL: {}\n\
                     smelt errors: {:?}",
                    sql, smelt_result.errors
                );
            }
        }
    }

    /// Standard SQL against all available references
    #[test]
    fn prop_all_references_equivalence(sql in generators::simple_select()) {
        if smelt_parser_compat::has_smelt_extensions(&sql) {
            return Ok(());
        }

        match compare_all_parse_results(&sql) {
            Ok(_) => (),
            Err(e) => prop_assert!(false, "{}", e),
        }
    }
}

// ===== Explicit Spark SQL unit tests =====

#[test]
fn test_spark_qualify_basic() {
    let sql = "SELECT * FROM users QUALIFY ROW_NUMBER() OVER (ORDER BY id) = 1";
    let smelt = SmeltParseResult::parse(sql);
    assert!(
        smelt.success,
        "smelt should parse QUALIFY: {:?}",
        smelt.errors
    );

    let spark = SparkSqlparserResult::parse(sql);
    assert!(
        spark.success,
        "sqlparser-databricks should parse QUALIFY: {:?}",
        spark.error
    );
}

#[test]
fn test_spark_qualify_with_partition() {
    let sql = "SELECT * FROM users QUALIFY ROW_NUMBER() OVER (PARTITION BY dept ORDER BY salary DESC) = 1";
    let smelt = SmeltParseResult::parse(sql);
    assert!(
        smelt.success,
        "smelt should parse QUALIFY with PARTITION BY: {:?}",
        smelt.errors
    );

    let spark = SparkSqlparserResult::parse(sql);
    assert!(
        spark.success,
        "sqlparser-databricks should parse QUALIFY with PARTITION BY: {:?}",
        spark.error
    );
}

#[test]
fn test_spark_transform_lambda() {
    let sql = "SELECT TRANSFORM(arr, x -> x + 1) FROM t";
    let smelt = SmeltParseResult::parse(sql);
    assert!(
        smelt.success,
        "smelt should parse TRANSFORM lambda: {:?}",
        smelt.errors
    );
}

#[test]
fn test_spark_aggregate_lambda() {
    let sql = "SELECT AGGREGATE(arr, 0, (acc, x) -> acc + x) FROM t";
    let smelt = SmeltParseResult::parse(sql);
    assert!(
        smelt.success,
        "smelt should parse AGGREGATE lambda: {:?}",
        smelt.errors
    );
}

#[test]
fn test_spark_pivot() {
    let sql = "SELECT * FROM sales PIVOT (SUM(amount) FOR quarter IN ('Q1', 'Q2', 'Q3', 'Q4'))";
    let smelt = SmeltParseResult::parse(sql);
    assert!(
        smelt.success,
        "smelt should parse PIVOT: {:?}",
        smelt.errors
    );
}

#[test]
fn test_spark_unpivot() {
    let sql = "SELECT * FROM sales UNPIVOT (val FOR quarter IN (q1, q2, q3, q4))";
    let smelt = SmeltParseResult::parse(sql);
    assert!(
        smelt.success,
        "smelt should parse UNPIVOT: {:?}",
        smelt.errors
    );
}

#[test]
fn test_spark_array_subscript() {
    let sql = "SELECT arr[1] FROM t";
    let smelt = SmeltParseResult::parse(sql);
    assert!(
        smelt.success,
        "smelt should parse array subscript: {:?}",
        smelt.errors
    );

    let spark = SparkSqlparserResult::parse(sql);
    assert!(
        spark.success,
        "sqlparser-databricks should parse array subscript: {:?}",
        spark.error
    );
}

#[test]
fn test_spark_array_slice() {
    let sql = "SELECT arr[1:5] FROM t";
    let smelt = SmeltParseResult::parse(sql);
    assert!(
        smelt.success,
        "smelt should parse array slice: {:?}",
        smelt.errors
    );
}
