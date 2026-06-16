//! Property-based test generators for SQL syntax
//!
//! This module provides proptest generators that create valid SQL queries
//! for round-trip testing and fuzzing.

use proptest::prelude::*;

// ===== Basic building blocks =====

/// SQL reserved keywords that must not be generated as bare identifiers.
///
/// This list covers the most common keywords that the smelt lexer assigns a
/// dedicated token (e.g. `IF_KW`, `AND`, `OR`) and therefore refuses to accept
/// as a plain `IDENT` in column/table positions.
const RESERVED_KEYWORDS: &[&str] = &[
    "and",
    "or",
    "not",
    "in",
    "is",
    "as",
    "on",
    "by",
    "to",
    "of",
    "at",
    "do",
    "if",
    "no",
    "up",
    "be",
    "go",
    "my",
    "us",
    "we",
    "all",
    "any",
    "are",
    "but",
    "can",
    "did",
    "end",
    "few",
    "for",
    "get",
    "got",
    "had",
    "has",
    "him",
    "his",
    "how",
    "its",
    "let",
    "may",
    "new",
    "now",
    "off",
    "old",
    "one",
    "our",
    "out",
    "own",
    "set",
    "she",
    "the",
    "two",
    "use",
    "was",
    "way",
    "who",
    "why",
    "yet",
    "you",
    "add",
    "asc",
    "avg",
    "bit",
    "day",
    "dec",
    "del",
    "div",
    "dup",
    "eof",
    "era",
    "fix",
    "get",
    "got",
    "hex",
    "key",
    "lag",
    "max",
    "min",
    "mod",
    "neg",
    "net",
    "nil",
    "nor",
    "not",
    "now",
    "null",
    "or",
    "ord",
    "out",
    "per",
    "raw",
    "rec",
    "ref",
    "row",
    "run",
    "sec",
    "sql",
    "sub",
    "sum",
    "sys",
    "tab",
    "top",
    "try",
    "uid",
    "url",
    "val",
    "var",
    "via",
    "win",
    "select",
    "from",
    "where",
    "join",
    "inner",
    "outer",
    "left",
    "right",
    "full",
    "cross",
    "on",
    "group",
    "order",
    "having",
    "limit",
    "offset",
    "distinct",
    "union",
    "intersect",
    "except",
    "with",
    "case",
    "when",
    "then",
    "else",
    "end",
    "between",
    "like",
    "ilike",
    "similar",
    "escape",
    "exists",
    "some",
    "every",
    "over",
    "partition",
    "rows",
    "range",
    "window",
    "interval",
    "current",
    "preceding",
    "following",
    "unbounded",
    "true",
    "false",
    "null",
];

/// Generate valid SQL identifiers that are not reserved keywords.
pub fn arb_identifier() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{2,10}".prop_filter("must not be a reserved keyword", |s| {
        !RESERVED_KEYWORDS.contains(&s.as_str())
    })
}

/// Generate valid SQL numbers
pub fn arb_number() -> impl Strategy<Value = String> {
    prop_oneof![
        // Integers
        (0i64..1000).prop_map(|n| n.to_string()),
        // Decimals
        (0.0..1000.0).prop_map(|n| format!("{:.2}", n)),
    ]
}

/// Generate valid SQL string literals
pub fn arb_string_literal() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_ ]{0,20}".prop_map(|s| format!("'{}'", s))
}

/// Generate simple column references
pub fn arb_column_ref() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple column
        arb_identifier(),
        // Qualified column (table.column)
        (arb_identifier(), arb_identifier()).prop_map(|(table, col)| format!("{}.{}", table, col)),
    ]
}

// ===== Expressions =====

/// Generate simple expressions (no recursion to avoid stack overflow)
pub fn arb_simple_expr() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_column_ref(),
        arb_number(),
        arb_string_literal(),
        Just("*".to_string()),
    ]
}

/// Generate binary operators
pub fn arb_binary_op() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("="),
        Just("!="),
        Just("<"),
        Just(">"),
        Just("<="),
        Just(">="),
        Just("+"),
        Just("-"),
        Just("*"),
        Just("/"),
        Just("%"),
        Just("AND"),
        Just("OR"),
    ]
    .prop_map(|s| s.to_string())
}

/// Generate comparison expressions (left op right)
pub fn arb_comparison_expr() -> impl Strategy<Value = String> {
    (arb_simple_expr(), arb_binary_op(), arb_simple_expr())
        .prop_map(|(left, op, right)| format!("{} {} {}", left, op, right))
}

/// Generate expressions with limited complexity
pub fn arb_expression() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => arb_simple_expr(),
        1 => arb_comparison_expr(),
    ]
}

// ===== Function calls =====

/// Generate simple function calls
pub fn arb_function_call() -> impl Strategy<Value = String> {
    let func_name = prop_oneof![
        Just("COUNT"),
        Just("SUM"),
        Just("AVG"),
        Just("MIN"),
        Just("MAX"),
    ];

    func_name.prop_flat_map(|name| {
        let name2 = name;
        prop_oneof![
            Just(format!("{}(*)", name)),
            arb_column_ref().prop_map(move |col| format!("{}({})", name2, col)),
        ]
    })
}

/// Generate smelt.ref() calls
pub fn arb_ref_call() -> impl Strategy<Value = String> {
    arb_identifier().prop_map(|model| format!("smelt.models.{}", model))
}

// ===== SELECT list =====

/// Generate a single SELECT item
pub fn arb_select_item() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => arb_simple_expr(),
        1 => arb_function_call(),
        // With alias: only alias non-star expressions, since `* AS alias` is invalid.
        1 => (arb_column_ref(), arb_identifier())
            .prop_map(|(expr, alias)| format!("{} AS {}", expr, alias)),
        1 => (arb_function_call(), arb_identifier())
            .prop_map(|(expr, alias)| format!("{} AS {}", expr, alias)),
    ]
}

/// Generate a SELECT list (comma-separated items)
pub fn arb_select_list() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_select_item(), 1..=5).prop_map(|items| items.join(", "))
}

// ===== Table references =====

/// Generate a table reference
pub fn arb_table_ref() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => arb_identifier(),
        1 => arb_ref_call(),
    ]
}

// ===== JOIN clauses =====

/// Generate JOIN types
pub fn arb_join_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("INNER JOIN"),
        Just("LEFT JOIN"),
        Just("RIGHT JOIN"),
        Just("FULL JOIN"),
        Just("CROSS JOIN"),
        Just("JOIN"), // Bare JOIN (defaults to INNER)
    ]
    .prop_map(|s| s.to_string())
}

/// Generate a simple JOIN clause (only ON conditions, not USING for simplicity)
pub fn arb_join_clause() -> impl Strategy<Value = String> {
    (arb_join_type(), arb_table_ref(), arb_comparison_expr()).prop_map(
        |(join_type, table, condition)| {
            if join_type == "CROSS JOIN" {
                format!("{} {}", join_type, table)
            } else {
                format!("{} {} ON {}", join_type, table, condition)
            }
        },
    )
}

// ===== WHERE clause =====

/// Generate a WHERE clause
pub fn arb_where_clause() -> impl Strategy<Value = String> {
    arb_expression().prop_map(|expr| format!("WHERE {}", expr))
}

// ===== GROUP BY clause =====

/// Generate a GROUP BY clause
pub fn arb_group_by_clause() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_column_ref(), 1..=3)
        .prop_map(|cols| format!("GROUP BY {}", cols.join(", ")))
}

// ===== HAVING clause =====

/// Generate a HAVING clause (similar to WHERE, but with aggregates)
#[allow(dead_code)] // Will be used for more complex queries in future
pub fn arb_having_clause() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_comparison_expr(),
        arb_function_call().prop_flat_map(|func| {
            (Just(func), arb_binary_op(), arb_number())
                .prop_map(|(f, op, n)| format!("{} {} {}", f, op, n))
        }),
    ]
    .prop_map(|expr| format!("HAVING {}", expr))
}

// ===== ORDER BY clause =====

/// Generate ORDER BY direction
pub fn arb_sort_direction() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("ASC"),
        Just("DESC"),
        Just(""), // No explicit direction
    ]
    .prop_map(|s| s.to_string())
}

/// Generate null ordering
pub fn arb_null_ordering() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("NULLS FIRST"),
        Just("NULLS LAST"),
        Just(""), // No null ordering
    ]
    .prop_map(|s| s.to_string())
}

/// Generate a single ORDER BY item
pub fn arb_order_by_item() -> impl Strategy<Value = String> {
    (arb_column_ref(), arb_sort_direction(), arb_null_ordering()).prop_map(|(col, dir, nulls)| {
        let mut parts = vec![col];
        if !dir.is_empty() {
            parts.push(dir);
        }
        if !nulls.is_empty() {
            parts.push(nulls);
        }
        parts.join(" ")
    })
}

/// Generate an ORDER BY clause
pub fn arb_order_by_clause() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_order_by_item(), 1..=3)
        .prop_map(|items| format!("ORDER BY {}", items.join(", ")))
}

// ===== LIMIT clause =====

/// Generate a LIMIT clause
pub fn arb_limit_clause() -> impl Strategy<Value = String> {
    prop_oneof![
        (1u32..100).prop_map(|n| format!("LIMIT {}", n)),
        (1u32..100, 0u32..50)
            .prop_map(|(limit, offset)| format!("LIMIT {} OFFSET {}", limit, offset)),
    ]
}

// ===== Complete SELECT statements =====

/// Generate a simple SELECT statement (SELECT ... FROM ...)
pub fn arb_simple_select() -> impl Strategy<Value = String> {
    (arb_select_list(), arb_table_ref())
        .prop_map(|(select_list, table)| format!("SELECT {} FROM {}", select_list, table))
}

/// Generate SELECT with WHERE
pub fn arb_select_with_where() -> impl Strategy<Value = String> {
    (arb_select_list(), arb_table_ref(), arb_where_clause()).prop_map(
        |(select_list, table, where_clause)| {
            format!("SELECT {} FROM {} {}", select_list, table, where_clause)
        },
    )
}

/// Generate SELECT with JOIN
pub fn arb_select_with_join() -> impl Strategy<Value = String> {
    (arb_select_list(), arb_table_ref(), arb_join_clause()).prop_map(
        |(select_list, table, join)| format!("SELECT {} FROM {} {}", select_list, table, join),
    )
}

/// Generate SELECT with GROUP BY
pub fn arb_select_with_group_by() -> impl Strategy<Value = String> {
    (arb_select_list(), arb_table_ref(), arb_group_by_clause()).prop_map(
        |(select_list, table, group_by)| {
            format!("SELECT {} FROM {} {}", select_list, table, group_by)
        },
    )
}

/// Generate SELECT with ORDER BY
pub fn arb_select_with_order_by() -> impl Strategy<Value = String> {
    (arb_select_list(), arb_table_ref(), arb_order_by_clause()).prop_map(
        |(select_list, table, order_by)| {
            format!("SELECT {} FROM {} {}", select_list, table, order_by)
        },
    )
}

/// Generate SELECT with LIMIT
pub fn arb_select_with_limit() -> impl Strategy<Value = String> {
    (arb_select_list(), arb_table_ref(), arb_limit_clause()).prop_map(
        |(select_list, table, limit)| format!("SELECT {} FROM {} {}", select_list, table, limit),
    )
}

/// Generate DISTINCT SELECT
pub fn arb_select_distinct() -> impl Strategy<Value = String> {
    (arb_select_list(), arb_table_ref())
        .prop_map(|(select_list, table)| format!("SELECT DISTINCT {} FROM {}", select_list, table))
}

/// Generate window spec names (simple identifiers used as window aliases)
#[allow(dead_code)]
pub fn arb_window_spec_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("w".to_string()),
        Just("w1".to_string()),
        Just("win".to_string()),
    ]
}

/// Generate a named window definition with varied frame shapes:
/// PARTITION-only, ORDER-only, PARTITION+ORDER, multiple columns.
///
/// The window alias is always `w` so that SELECT...OVER w and WINDOW w AS (...) agree.
pub fn arb_named_window_def() -> impl Strategy<Value = String> {
    (
        arb_column_ref(),
        arb_column_ref(),
        arb_column_ref(),
        arb_column_ref(),
    )
        .prop_flat_map(|(p1, p2, o1, o2)| {
            let p1b = p1.clone();
            let p2b = p2.clone();
            let o1b = o1.clone();
            let o2b = o2.clone();
            prop_oneof![
                // PARTITION BY only
                Just(format!("w AS (PARTITION BY {})", p1)),
                // ORDER BY only
                Just(format!("w AS (ORDER BY {})", o1b)),
                // PARTITION BY + ORDER BY (single cols)
                Just(format!("w AS (PARTITION BY {} ORDER BY {})", p1b, o1)),
                // PARTITION BY multiple + ORDER BY multiple
                Just(format!(
                    "w AS (PARTITION BY {}, {} ORDER BY {}, {})",
                    p2b, p2, o2b, o2
                )),
            ]
        })
}

/// Generate a SELECT with a named WINDOW clause using varied aggregate functions
/// and window frame shapes (PARTITION-only, ORDER-only, PARTITION+ORDER).
///
/// The window alias is always `w` in both `OVER w` and `WINDOW w AS (...)` to
/// avoid alias-mismatch parse errors.
pub fn arb_select_with_window_clause() -> impl Strategy<Value = String> {
    (
        arb_identifier(), // table
        arb_column_ref(), // column for aggregate
        arb_named_window_def(),
    )
        .prop_flat_map(|(table, col, window_def)| {
            let col2 = col.clone();
            let col3 = col.clone();
            let table2 = table.clone();
            let table3 = table.clone();
            let wd2 = window_def.clone();
            let wd3 = window_def.clone();
            prop_oneof![
                Just(format!(
                    "SELECT SUM({col}) OVER w FROM {table} WINDOW {window_def}"
                )),
                Just(format!(
                    "SELECT AVG({col2}) OVER w FROM {table2} WINDOW {wd2}"
                )),
                Just(format!(
                    "SELECT MAX({col3}) OVER w FROM {table3} WINDOW {wd3}"
                )),
            ]
        })
}

/// Generate valid INTERVAL unit keywords
pub fn arb_interval_unit() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("DAY"),
        Just("MONTH"),
        Just("YEAR"),
        Just("HOUR"),
        Just("MINUTE"),
        Just("SECOND"),
        Just("WEEK"),
    ]
    .prop_map(|s| s.to_string())
}

/// Generate a numeric INTERVAL expression in one of three forms:
/// - `INTERVAL n UNIT`
/// - `INTERVAL (n) UNIT`
/// - `n * INTERVAL 1 UNIT`
pub fn arb_numeric_interval() -> impl Strategy<Value = String> {
    (1u32..30, arb_interval_unit()).prop_flat_map(|(n, unit)| {
        let unit2 = unit.clone();
        let unit3 = unit.clone();
        prop_oneof![
            Just(format!("INTERVAL {} {}", n, unit)),
            Just(format!("INTERVAL ({}) {}", n, unit2)),
            Just(format!("{} * INTERVAL 1 {}", n, unit3)),
        ]
    })
}

/// Generate a SELECT using a numeric INTERVAL in an expression
/// Example: `SELECT col + INTERVAL 1 DAY AS alias FROM t`
pub fn arb_select_with_interval() -> impl Strategy<Value = String> {
    (
        arb_identifier(),
        arb_column_ref(),
        arb_numeric_interval(),
        arb_identifier(),
    )
        .prop_map(|(table, col, interval, alias)| {
            format!("SELECT {} + {} AS {} FROM {}", col, interval, alias, table)
        })
}

/// Generate any "base" SELECT (no CTE wrapping) — used as the inner of CTE generators
/// to avoid infinite recursion.
pub fn arb_any_select_basic() -> impl Strategy<Value = String> {
    prop_oneof![
        2 => arb_simple_select(),
        1 => arb_select_with_where(),
        1 => arb_select_with_join(),
        1 => arb_select_with_group_by(),
        1 => arb_select_with_order_by(),
        1 => arb_select_with_limit(),
        1 => arb_select_distinct(),
        1 => arb_select_with_window_clause(),
        1 => arb_select_with_interval(),
    ]
}

/// Generate a CTE-wrapped SELECT supporting 1 or 2 CTE levels and non-`SELECT *` outer.
///
/// Uses `arb_simple_select()` (not `arb_any_select()`) for the inner to keep the
/// strategy tree shallow and avoid stack overflows in debug builds.
pub fn arb_cte_wrapped_select() -> impl Strategy<Value = String> {
    (arb_simple_select(), arb_select_list(), 0usize..3usize).prop_map(
        |(inner, outer_list, variant)| match variant {
            // 1-level, SELECT *
            0 => format!("WITH cte AS ({inner}) SELECT * FROM cte"),
            // 1-level, non-SELECT * outer projection
            1 => format!("WITH cte AS ({inner}) SELECT {outer_list} FROM cte"),
            // 2-level chained CTEs (use a fixed inner for the second level)
            _ => format!("WITH a AS ({inner}), b AS (SELECT * FROM a) SELECT {outer_list} FROM b"),
        },
    )
}

/// Generate a CTE whose inner SELECT has a WINDOW clause
pub fn arb_cte_with_window() -> impl Strategy<Value = String> {
    arb_select_with_window_clause()
        .prop_map(|inner| format!("WITH cte AS ({}) SELECT * FROM cte", inner))
}

/// Generate a CTE whose inner SELECT uses numeric INTERVAL
pub fn arb_cte_with_interval() -> impl Strategy<Value = String> {
    arb_select_with_interval()
        .prop_map(|inner| format!("WITH cte AS ({}) SELECT * FROM cte", inner))
}

/// Generate any valid SELECT statement, including CTE-wrapped forms.
pub fn arb_any_select() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => arb_any_select_basic(),
        1 => arb_cte_wrapped_select(),
        1 => arb_cte_with_window(),
        1 => arb_cte_with_interval(),
    ]
}
