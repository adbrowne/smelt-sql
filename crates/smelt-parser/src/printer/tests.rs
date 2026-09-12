use super::*;
use crate::parse;

fn assert_round_trip(sql: &str) {
    let parse1 = parse(sql);
    assert_eq!(parse1.errors.len(), 0, "Parse errors: {:?}", parse1.errors);

    let file = File::cast(parse1.syntax()).unwrap();
    let printed = file.to_string();

    let parse2 = parse(&printed);
    assert_eq!(
        parse2.errors.len(),
        0,
        "Re-parse errors: {:?}\nPrinted SQL: {}",
        parse2.errors,
        printed
    );

    // For debugging: print both versions
    if printed.trim() != sql.trim() {
        eprintln!("Original: {}", sql);
        eprintln!("Printed:  {}", printed);
    }
}

#[test]
fn test_simple_select() {
    assert_round_trip("SELECT * FROM users");
}

#[test]
fn test_select_with_alias() {
    assert_round_trip("SELECT name AS user_name FROM users");
}

#[test]
fn test_select_join() {
    assert_round_trip("SELECT * FROM users INNER JOIN orders ON users.id = orders.user_id");
}

#[test]
fn test_comma_join_two_tables() {
    assert_round_trip("SELECT * FROM users, orders");
}

#[test]
fn test_comma_join_three_tables() {
    assert_round_trip("SELECT * FROM a, b, c");
}

#[test]
fn test_comma_join_mixed_with_explicit_join() {
    assert_round_trip("SELECT * FROM a, b JOIN c ON b.id = c.id");
}

#[test]
fn test_comma_join_with_aliases() {
    assert_round_trip("SELECT * FROM users AS u, orders AS o");
}

#[test]
fn test_select_where() {
    assert_round_trip("SELECT * FROM users WHERE age > 18");
}

#[test]
fn test_select_order_by() {
    assert_round_trip("SELECT * FROM users ORDER BY name ASC");
}

#[test]
fn test_select_limit() {
    assert_round_trip("SELECT * FROM users LIMIT 10");
}

#[test]
fn test_select_cte() {
    assert_round_trip("WITH active_users AS (SELECT * FROM users WHERE status = 'active') SELECT * FROM active_users");
}

#[test]
fn test_select_window_function() {
    assert_round_trip("SELECT ROW_NUMBER() OVER (ORDER BY created_at) FROM events");
}

#[test]
fn test_select_distinct() {
    assert_round_trip("SELECT DISTINCT city FROM users");
}

#[test]
fn test_select_group_by_having() {
    assert_round_trip("SELECT city, COUNT(*) FROM users GROUP BY city HAVING COUNT(*) > 5");
}

#[test]
fn test_round_trip_mixed_case_where() {
    // Regression test for fuzzer crash with mixed-case WHERE keyword
    assert_round_trip("SELECT x FROM t WHERE y = 1");
}

// ===== Mixed-case keyword regression tests =====
// These tests verify that round-trip works correctly regardless of keyword casing.
// SQL keywords are case-insensitive, so the parser accepts any casing.
// The printer normalizes to uppercase.

#[test]
fn test_mixed_case_where_actual() {
    // The original bug: mixed-case WHERE like "WhErE" would crash
    assert_round_trip("SELECT x FROM t WhErE y = 1");
}

#[test]
fn test_mixed_case_select() {
    assert_round_trip("sElEcT * FROM users");
}

#[test]
fn test_mixed_case_from() {
    assert_round_trip("SELECT * fRoM users");
}

#[test]
fn test_mixed_case_inner_join() {
    assert_round_trip("SELECT * FROM a InNeR jOiN b ON a.id = b.id");
}

#[test]
fn test_mixed_case_left_join() {
    assert_round_trip("SELECT * FROM a LeFt JoIn b ON a.id = b.id");
}

#[test]
fn test_mixed_case_group_by() {
    assert_round_trip("SELECT city FROM users GrOuP bY city");
}

#[test]
fn test_mixed_case_order_by() {
    assert_round_trip("SELECT * FROM users OrDeR bY name");
}

#[test]
fn test_mixed_case_order_by_asc_desc() {
    assert_round_trip("SELECT * FROM users ORDER BY name AsC, age DeSc");
}

#[test]
fn test_mixed_case_having() {
    assert_round_trip("SELECT city, COUNT(*) FROM users GROUP BY city HaViNg COUNT(*) > 5");
}

#[test]
fn test_mixed_case_limit() {
    assert_round_trip("SELECT * FROM users LiMiT 10");
}

#[test]
fn test_mixed_case_limit_offset() {
    assert_round_trip("SELECT * FROM users LIMIT 10 oFfSeT 5");
}

#[test]
fn test_mixed_case_distinct() {
    assert_round_trip("SELECT DiStInCt city FROM users");
}

#[test]
fn test_mixed_case_with_cte() {
    assert_round_trip("WiTh cte aS (SELECT 1) SELECT * FROM cte");
}

#[test]
fn test_mixed_case_on_using() {
    assert_round_trip("SELECT * FROM a JOIN b On a.id = b.id");
    assert_round_trip("SELECT * FROM a JOIN b UsInG (id)");
}

#[test]
fn test_mixed_case_nulls_first_last() {
    assert_round_trip("SELECT * FROM users ORDER BY name NuLlS FiRsT");
    assert_round_trip("SELECT * FROM users ORDER BY name NULLS lAsT");
}

#[test]
fn test_mixed_case_and_or() {
    assert_round_trip("SELECT * FROM users WHERE a = 1 AnD b = 2 oR c = 3");
}

// QUALIFY round-trip
#[test]
fn test_qualify_round_trip() {
    assert_round_trip("SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1");
}

#[test]
fn test_qualify_with_having_round_trip() {
    assert_round_trip("SELECT city, COUNT(*) FROM t GROUP BY city HAVING COUNT(*) > 1 QUALIFY ROW_NUMBER() OVER (ORDER BY city) = 1");
}

// Array subscript round-trip
#[test]
fn test_array_subscript_round_trip() {
    assert_round_trip("SELECT arr[1] FROM t");
}

#[test]
fn test_array_slice_round_trip() {
    assert_round_trip("SELECT arr[1:3] FROM t");
}

#[test]
fn test_date_literal_round_trip() {
    assert_round_trip("SELECT * FROM t WHERE d = DATE '2024-01-01'");
}

// UNION ALL printing test
#[test]
fn test_union_all_round_trip() {
    assert_round_trip("SELECT id FROM a UNION ALL SELECT id FROM b");
}

#[test]
fn test_union_round_trip() {
    assert_round_trip("SELECT id FROM a UNION SELECT id FROM b");
}

// NULLS FIRST/LAST printing tests
#[test]
fn test_nulls_first_round_trip() {
    assert_round_trip("SELECT * FROM t ORDER BY name NULLS FIRST");
}

#[test]
fn test_nulls_last_round_trip() {
    assert_round_trip("SELECT * FROM t ORDER BY name DESC NULLS LAST");
}

// INTERSECT / EXCEPT round-trip
#[test]
fn test_intersect_round_trip() {
    assert_round_trip("SELECT id FROM a INTERSECT SELECT id FROM b");
}

#[test]
fn test_except_round_trip() {
    assert_round_trip("SELECT id FROM a EXCEPT SELECT id FROM b");
}

#[test]
fn test_intersect_all_round_trip() {
    assert_round_trip("SELECT id FROM a INTERSECT ALL SELECT id FROM b");
}

#[test]
fn test_except_all_round_trip() {
    assert_round_trip("SELECT id FROM a EXCEPT ALL SELECT id FROM b");
}

// Regression test for a `round_trip` fuzz crash: a SELECT item ending in
// an unterminated `--` line comment must keep the newline that
// terminates it when printed, or the following clause keyword gets
// silently swallowed into the comment on re-parse.
#[test]
fn test_select_item_trailing_line_comment_before_having_round_trip() {
    assert_round_trip("SELECT x --comment\nHAVING y > 1");
}

#[test]
fn test_select_star_trailing_line_comment_before_from_round_trip() {
    assert_round_trip("SELECT * --comment\nFROM t");
}
