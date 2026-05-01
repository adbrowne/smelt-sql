//! Property-based test generators for SQL syntax
//!
//! This module provides proptest generators that create valid SQL queries
//! for round-trip testing and fuzzing.

use proptest::prelude::*;

// ===== Basic building blocks =====

/// Generate valid SQL identifiers
pub fn arb_identifier() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,10}".prop_map(|s| s)
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
        // With alias
        1 => (arb_simple_expr(), arb_identifier())
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

/// Generate any valid SELECT statement
pub fn arb_any_select() -> impl Strategy<Value = String> {
    prop_oneof![
        2 => arb_simple_select(),
        1 => arb_select_with_where(),
        1 => arb_select_with_join(),
        1 => arb_select_with_group_by(),
        1 => arb_select_with_order_by(),
        1 => arb_select_with_limit(),
        1 => arb_select_distinct(),
    ]
}
