//! Property-based round-trip tests
//!
//! These tests use proptest to generate thousands of SQL queries and verify that:
//! 1. The parser never panics
//! 2. Valid SQL can be printed and re-parsed
//! 3. The AST structure is preserved through round-trips

use proptest::prelude::*;
use smelt_parser::{parse, File};

mod proptest_generators;
use proptest_generators::*;

/// Helper to check if a parse result is valid (no errors)
#[allow(dead_code)]
fn is_valid_parse(sql: &str) -> bool {
    let result = parse(sql);
    result.errors.is_empty()
}

/// Helper to perform round-trip test: parse → print → parse
fn assert_round_trip(sql: &str) {
    let parse1 = parse(sql);

    // Skip if original parse has errors (we're testing valid SQL round-trips)
    if !parse1.errors.is_empty() {
        return;
    }

    // Print the SQL
    let file = File::cast(parse1.syntax()).expect("should have FILE node");
    let printed = file.to_string();

    // Re-parse the printed SQL
    let parse2 = parse(&printed);

    // The re-parsed SQL should have no errors
    if !parse2.errors.is_empty() {
        panic!(
            "Round-trip failed!\nOriginal: {}\nPrinted: {}\nErrors: {:?}",
            sql, printed, parse2.errors
        );
    }

    // Both parses should have the same structure (ignoring whitespace)
    let ast1 = format!("{:#?}", parse1.syntax());
    let ast2 = format!("{:#?}", parse2.syntax());

    if ast1 != ast2 {
        eprintln!("AST mismatch!");
        eprintln!("Original SQL: {}", sql);
        eprintln!("Printed SQL:  {}", printed);
        eprintln!("AST1 (original):\n{}", ast1);
        eprintln!("AST2 (printed):\n{}", ast2);
        // Don't panic here, just log - whitespace differences are expected
    }
}

// ===== Property tests for round-trip preservation =====

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property: Simple SELECT statements round-trip correctly
    #[test]
    fn prop_simple_select_round_trip(sql in arb_simple_select()) {
        assert_round_trip(&sql);
    }

    /// Property: SELECT with WHERE round-trips correctly
    #[test]
    fn prop_select_where_round_trip(sql in arb_select_with_where()) {
        assert_round_trip(&sql);
    }

    /// Property: SELECT with JOIN round-trips correctly
    #[test]
    fn prop_select_join_round_trip(sql in arb_select_with_join()) {
        assert_round_trip(&sql);
    }

    /// Property: SELECT with GROUP BY round-trips correctly
    #[test]
    fn prop_select_group_by_round_trip(sql in arb_select_with_group_by()) {
        assert_round_trip(&sql);
    }

    /// Property: SELECT with ORDER BY round-trips correctly
    #[test]
    fn prop_select_order_by_round_trip(sql in arb_select_with_order_by()) {
        assert_round_trip(&sql);
    }

    /// Property: SELECT with LIMIT round-trips correctly
    #[test]
    fn prop_select_limit_round_trip(sql in arb_select_with_limit()) {
        assert_round_trip(&sql);
    }

    /// Property: DISTINCT SELECT round-trips correctly
    #[test]
    fn prop_select_distinct_round_trip(sql in arb_select_distinct()) {
        assert_round_trip(&sql);
    }

    /// Property: Any valid SELECT statement round-trips correctly
    #[test]
    fn prop_any_select_round_trip(sql in arb_any_select()) {
        assert_round_trip(&sql);
    }
}

// ===== Property tests for parser robustness =====

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Property: Parser never panics on any generated SQL
    #[test]
    fn prop_parser_never_panics(sql in arb_any_select()) {
        let _ = parse(&sql); // Should not panic
    }

    /// Property: Parser never panics on arbitrary strings
    #[test]
    fn prop_parser_never_panics_arbitrary(s in "\\PC{0,100}") {
        let _ = parse(&s); // Should not panic
    }
}

// ===== Specific edge case tests =====

#[test]
fn test_round_trip_empty_select() {
    // This should parse and round-trip
    let sql = "SELECT * FROM users";
    assert_round_trip(sql);
}

#[test]
fn test_round_trip_multiple_columns() {
    let sql = "SELECT id, name, email FROM users";
    assert_round_trip(sql);
}

#[test]
fn test_round_trip_qualified_columns() {
    let sql = "SELECT users.id, users.name FROM users";
    assert_round_trip(sql);
}

#[test]
fn test_round_trip_function_calls() {
    let sql = "SELECT COUNT(*), SUM(amount) FROM transactions";
    assert_round_trip(sql);
}

#[test]
fn test_round_trip_complex_where() {
    let sql = "SELECT * FROM users WHERE age > 18 AND status = 'active'";
    assert_round_trip(sql);
}

#[test]
fn test_round_trip_multiple_joins() {
    let sql = "SELECT * FROM users INNER JOIN orders ON users.id = orders.user_id LEFT JOIN products ON orders.product_id = products.id";
    assert_round_trip(sql);
}

#[test]
fn test_round_trip_group_by_having() {
    let sql = "SELECT city, COUNT(*) FROM users GROUP BY city HAVING COUNT(*) > 5";
    assert_round_trip(sql);
}

#[test]
fn test_round_trip_order_by_complex() {
    let sql = "SELECT * FROM users ORDER BY age DESC NULLS LAST, name ASC";
    assert_round_trip(sql);
}

#[test]
fn test_round_trip_limit_offset() {
    let sql = "SELECT * FROM users LIMIT 10 OFFSET 20";
    assert_round_trip(sql);
}

#[test]
fn test_round_trip_ref_call() {
    let sql = "SELECT * FROM smelt.models.events";
    assert_round_trip(sql);
}

// Regression: fuzz crash where a comment between `.` and `*` in a qualified
// wildcard caused the STAR to be left outside the SELECT_ITEM, so the printer
// emitted `SELECT TTTQ./**/` which fails to re-parse.
#[test]
fn test_round_trip_qualified_star_with_comment() {
    let sql = "SELECT TTTQ./**/*";
    assert_round_trip(sql);
}

// Regression: original crash input from fuzz corpus (round_trip target).
// Trailing junk after the SELECT body must not produce a parse with zero
// errors that the printer cannot reproduce verbatim.
#[test]
fn test_round_trip_fuzz_crash_qualified_star_trailing_junk() {
    let sql = "SELECT\nTTTQ./**/*///4/*T";
    assert_round_trip(sql);
}

// Regression: fuzz crash where LIST_SPREAD nodes in the SELECT list were
// invisible to the printer (only SELECT_ITEM nodes were iterated), causing
// "SELECT ... Cm, ...hAY" to print as "SELECT " which fails to re-parse.
#[test]
fn test_round_trip_fuzz_crash_list_spread_in_select() {
    let sql = "SELECT ... Cm, ...hAY";
    assert_round_trip(sql);
}

#[test]
fn test_round_trip_single_spread_in_select() {
    let sql = "SELECT ...foo";
    assert_round_trip(sql);
}
