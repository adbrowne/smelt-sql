//! PostgreSQL grammar-based SQL generators
//!
//! This module provides proptest strategies for generating valid PostgreSQL SELECT queries.
//! The generators are designed to exercise various SQL features to find parser gaps.

use proptest::prelude::*;

/// Generate a simple identifier
pub fn identifier() -> impl Strategy<Value = String> {
    // Start with letter, followed by letters/numbers/underscore
    // Avoid SQL keywords
    "[a-z][a-z0-9_]{0,7}".prop_filter("avoid keywords", |s| !is_keyword(s))
}

/// Check if a string is a SQL keyword
fn is_keyword(s: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "select",
        "from",
        "where",
        "and",
        "or",
        "not",
        "as",
        "on",
        "join",
        "inner",
        "left",
        "right",
        "full",
        "cross",
        "outer",
        "group",
        "by",
        "having",
        "order",
        "limit",
        "offset",
        "union",
        "all",
        "distinct",
        "case",
        "when",
        "then",
        "else",
        "end",
        "cast",
        "in",
        "between",
        "is",
        "null",
        "true",
        "false",
        "asc",
        "desc",
        "nulls",
        "first",
        "last",
        "with",
        "recursive",
        "over",
        "partition",
        "rows",
        "range",
        "groups",
        "unbounded",
        "preceding",
        "following",
        "current",
        "row",
        "filter",
        "exists",
        "using",
        "lateral",
        // PostgreSQL reserved words that are short identifiers
        "do",
        "to",
        "if",
        "no",
        "of",
    ];
    KEYWORDS.contains(&s.to_lowercase().as_str())
}

/// Generate a table name
pub fn table_name() -> impl Strategy<Value = String> {
    identifier()
}

/// Generate a column name
pub fn column_name() -> impl Strategy<Value = String> {
    identifier()
}

/// Generate an alias
pub fn alias() -> impl Strategy<Value = String> {
    identifier()
}

/// Generate a numeric literal
pub fn numeric_literal() -> impl Strategy<Value = String> {
    prop_oneof![
        (0i64..1000).prop_map(|n| n.to_string()),
        (0.0f64..1000.0).prop_map(|f| format!("{:.2}", f)),
    ]
}

/// Generate a string literal
pub fn string_literal() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_ ]{0,20}".prop_map(|s| format!("'{}'", s.replace('\'', "''")))
}

/// Generate a simple expression (column or literal)
pub fn simple_expr() -> impl Strategy<Value = String> {
    // Note: Don't include "*" here as it's only valid in SELECT list or COUNT(*)
    // PostgreSQL rejects * in expressions like "a + *" or "* = a"
    prop_oneof![column_name(), numeric_literal(), string_literal(),]
}

/// Generate a comparison operator
pub fn comparison_op() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("=".to_string()),
        Just("<>".to_string()),
        Just("!=".to_string()),
        Just("<".to_string()),
        Just(">".to_string()),
        Just("<=".to_string()),
        Just(">=".to_string()),
    ]
}

/// Generate an arithmetic operator
pub fn arithmetic_op() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("+".to_string()),
        Just("-".to_string()),
        Just("*".to_string()),
        Just("/".to_string()),
    ]
}

/// Generate a binary expression
pub fn binary_expr() -> impl Strategy<Value = String> {
    (simple_expr(), arithmetic_op(), simple_expr())
        .prop_map(|(l, op, r)| format!("{} {} {}", l, op, r))
}

/// Generate an expression (simple or binary)
pub fn expression() -> impl Strategy<Value = String> {
    prop_oneof![simple_expr(), binary_expr(),]
}

/// Generate a comparison expression for WHERE clauses
pub fn comparison_expr() -> impl Strategy<Value = String> {
    (simple_expr(), comparison_op(), simple_expr())
        .prop_map(|(l, op, r)| format!("{} {} {}", l, op, r))
}

/// Generate a function call
pub fn function_call() -> impl Strategy<Value = String> {
    let funcs = prop_oneof![
        Just("COUNT".to_string()),
        Just("SUM".to_string()),
        Just("AVG".to_string()),
        Just("MIN".to_string()),
        Just("MAX".to_string()),
        Just("COALESCE".to_string()),
        Just("UPPER".to_string()),
        Just("LOWER".to_string()),
        Just("LENGTH".to_string()),
    ];

    (funcs, expression()).prop_map(|(func, arg)| format!("{}({})", func, arg))
}

/// Generate a select item (expression with optional alias)
pub fn select_item() -> impl Strategy<Value = String> {
    prop_oneof![
        expression(),
        function_call(),
        (expression(), alias()).prop_map(|(e, a)| format!("{} AS {}", e, a)),
        (function_call(), alias()).prop_map(|(f, a)| format!("{} AS {}", f, a)),
    ]
}

/// Generate a select list
pub fn select_list() -> impl Strategy<Value = String> {
    prop::collection::vec(select_item(), 1..=5).prop_map(|items| items.join(", "))
}

/// Generate an optional WHERE clause
pub fn where_clause() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        comparison_expr().prop_map(|e| Some(format!("WHERE {}", e))),
        (comparison_expr(), comparison_expr())
            .prop_map(|(e1, e2)| Some(format!("WHERE {} AND {}", e1, e2))),
    ]
}

/// Generate an optional GROUP BY clause
pub fn group_by_clause() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        column_name().prop_map(|c| Some(format!("GROUP BY {}", c))),
        (column_name(), column_name())
            .prop_map(|(c1, c2)| Some(format!("GROUP BY {}, {}", c1, c2))),
    ]
}

/// Generate an optional HAVING clause (requires GROUP BY)
pub fn having_clause() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        comparison_expr().prop_map(|e| Some(format!("HAVING {}", e))),
    ]
}

/// Generate a sort direction
pub fn sort_direction() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("".to_string()),
        Just(" ASC".to_string()),
        Just(" DESC".to_string()),
    ]
}

/// Generate a null ordering
pub fn null_ordering() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("".to_string()),
        Just(" NULLS FIRST".to_string()),
        Just(" NULLS LAST".to_string()),
    ]
}

/// Generate an ORDER BY item
pub fn order_by_item() -> impl Strategy<Value = String> {
    (column_name(), sort_direction(), null_ordering())
        .prop_map(|(c, dir, nulls)| format!("{}{}{}", c, dir, nulls))
}

/// Generate an optional ORDER BY clause
pub fn order_by_clause() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        order_by_item().prop_map(|i| Some(format!("ORDER BY {}", i))),
        (order_by_item(), order_by_item())
            .prop_map(|(i1, i2)| Some(format!("ORDER BY {}, {}", i1, i2))),
    ]
}

/// Generate an optional LIMIT clause
pub fn limit_clause() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        (1u32..1000).prop_map(|n| Some(format!("LIMIT {}", n))),
        Just(Some("LIMIT ALL".to_string())),
    ]
}

/// Generate an optional OFFSET clause
pub fn offset_clause() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        (0u32..100).prop_map(|n| Some(format!("OFFSET {}", n))),
    ]
}

/// Generate a JOIN type
pub fn join_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("JOIN".to_string()),
        Just("INNER JOIN".to_string()),
        Just("LEFT JOIN".to_string()),
        Just("LEFT OUTER JOIN".to_string()),
        Just("RIGHT JOIN".to_string()),
        Just("RIGHT OUTER JOIN".to_string()),
        Just("FULL JOIN".to_string()),
        Just("FULL OUTER JOIN".to_string()),
        Just("CROSS JOIN".to_string()),
    ]
}

/// Generate a JOIN condition
pub fn join_condition() -> impl Strategy<Value = String> {
    prop_oneof![
        (column_name(), column_name()).prop_map(|(c1, c2)| format!("ON t1.{} = t2.{}", c1, c2)),
        column_name().prop_map(|c| format!("USING ({})", c)),
    ]
}

/// Generate a simple SELECT statement
pub fn simple_select() -> impl Strategy<Value = String> {
    (
        select_list(),
        table_name(),
        where_clause(),
        order_by_clause(),
        limit_clause(),
    )
        .prop_map(|(sel, tbl, whr, ord, lim)| {
            let mut sql = format!("SELECT {} FROM {}", sel, tbl);
            if let Some(w) = whr {
                sql.push_str(&format!(" {}", w));
            }
            if let Some(o) = ord {
                sql.push_str(&format!(" {}", o));
            }
            if let Some(l) = lim {
                sql.push_str(&format!(" {}", l));
            }
            sql
        })
}

/// Generate a SELECT with GROUP BY
pub fn select_with_group_by() -> impl Strategy<Value = String> {
    (
        function_call(),
        column_name(),
        table_name(),
        where_clause(),
        having_clause(),
    )
        .prop_map(|(agg, group_col, tbl, whr, hav)| {
            let mut sql = format!(
                "SELECT {}, {} FROM {} GROUP BY {}",
                group_col, agg, tbl, group_col
            );
            if let Some(w) = whr {
                sql = format!(
                    "SELECT {}, {} FROM {} {} GROUP BY {}",
                    group_col, agg, tbl, w, group_col
                );
            }
            if let Some(h) = hav {
                sql.push_str(&format!(" {}", h));
            }
            sql
        })
}

/// Generate a SELECT with JOIN
pub fn select_with_join() -> impl Strategy<Value = String> {
    (
        table_name(),
        table_name(),
        join_type(),
        join_condition(),
        where_clause(),
    )
        .prop_filter_map("valid join", |(t1, t2, jt, jc, whr)| {
            // CROSS JOIN doesn't need ON/USING
            if jt.contains("CROSS") {
                let mut sql = format!("SELECT * FROM {} {} {}", t1, jt, t2);
                if let Some(w) = whr {
                    sql.push_str(&format!(" {}", w));
                }
                Some(sql)
            } else {
                let mut sql = format!("SELECT * FROM {} AS t1 {} {} AS t2 {}", t1, jt, t2, jc);
                if let Some(w) = whr {
                    sql.push_str(&format!(" {}", w));
                }
                Some(sql)
            }
        })
}

/// Generate a DISTINCT SELECT
pub fn select_distinct() -> impl Strategy<Value = String> {
    (select_list(), table_name())
        .prop_map(|(sel, tbl)| format!("SELECT DISTINCT {} FROM {}", sel, tbl))
}

/// Generate a SELECT with CTE
pub fn select_with_cte() -> impl Strategy<Value = String> {
    (alias(), table_name(), column_name()).prop_map(|(cte_name, tbl, col)| {
        format!(
            "WITH {} AS (SELECT {} FROM {}) SELECT * FROM {}",
            cte_name, col, tbl, cte_name
        )
    })
}

/// Generate a window function
pub fn window_function() -> impl Strategy<Value = String> {
    let window_funcs = prop_oneof![
        Just("ROW_NUMBER()".to_string()),
        Just("RANK()".to_string()),
        Just("DENSE_RANK()".to_string()),
        (Just("SUM".to_string()), column_name()).prop_map(|(f, c)| format!("{}({})", f, c)),
        (Just("AVG".to_string()), column_name()).prop_map(|(f, c)| format!("{}({})", f, c)),
        (Just("COUNT".to_string()), column_name()).prop_map(|(f, c)| format!("{}({})", f, c)),
    ];

    let partition_clause = prop_oneof![
        Just("".to_string()),
        column_name().prop_map(|c| format!("PARTITION BY {} ", c)),
    ];

    let order_clause = column_name().prop_map(|c| format!("ORDER BY {}", c));

    (window_funcs, partition_clause, order_clause)
        .prop_map(|(func, part, ord)| format!("{} OVER ({}{})", func, part, ord))
}

/// Generate a SELECT with window function
pub fn select_with_window() -> impl Strategy<Value = String> {
    (column_name(), window_function(), table_name())
        .prop_map(|(col, wf, tbl)| format!("SELECT {}, {} FROM {}", col, wf, tbl))
}

/// Generate a UNION SELECT
pub fn select_union() -> impl Strategy<Value = String> {
    (column_name(), table_name(), table_name()).prop_map(|(col, t1, t2)| {
        format!(
            "SELECT {} FROM {} UNION SELECT {} FROM {}",
            col, t1, col, t2
        )
    })
}

/// Generate a UNION ALL SELECT
pub fn select_union_all() -> impl Strategy<Value = String> {
    (column_name(), table_name(), table_name()).prop_map(|(col, t1, t2)| {
        format!(
            "SELECT {} FROM {} UNION ALL SELECT {} FROM {}",
            col, t1, col, t2
        )
    })
}

/// Generate a CASE expression
pub fn case_expression() -> impl Strategy<Value = String> {
    prop_oneof![
        // Searched CASE
        (comparison_expr(), simple_expr(), simple_expr()).prop_map(|(cond, then_val, else_val)| {
            format!("CASE WHEN {} THEN {} ELSE {} END", cond, then_val, else_val)
        }),
        // Simple CASE
        (column_name(), simple_expr(), simple_expr(), simple_expr()).prop_map(
            |(col, val, then_val, else_val)| {
                format!(
                    "CASE {} WHEN {} THEN {} ELSE {} END",
                    col, val, then_val, else_val
                )
            }
        ),
    ]
}

/// Generate a SELECT with CASE
pub fn select_with_case() -> impl Strategy<Value = String> {
    (case_expression(), alias(), table_name())
        .prop_map(|(case_expr, a, tbl)| format!("SELECT {} AS {} FROM {}", case_expr, a, tbl))
}

/// Generate a subquery
pub fn subquery() -> impl Strategy<Value = String> {
    (column_name(), table_name()).prop_map(|(col, tbl)| format!("(SELECT {} FROM {})", col, tbl))
}

/// Generate a SELECT with IN subquery
pub fn select_with_in_subquery() -> impl Strategy<Value = String> {
    (column_name(), table_name(), subquery())
        .prop_map(|(col, tbl, sub)| format!("SELECT * FROM {} WHERE {} IN {}", tbl, col, sub))
}

/// Generate a SELECT with EXISTS
pub fn select_with_exists() -> impl Strategy<Value = String> {
    (table_name(), table_name(), column_name()).prop_map(|(t1, t2, col)| {
        format!(
            "SELECT * FROM {} AS a WHERE EXISTS (SELECT 1 FROM {} AS b WHERE a.{} = b.{})",
            t1, t2, col, col
        )
    })
}

/// Generate a SELECT with BETWEEN
pub fn select_with_between() -> impl Strategy<Value = String> {
    (
        column_name(),
        table_name(),
        numeric_literal(),
        numeric_literal(),
    )
        .prop_map(|(col, tbl, low, high)| {
            format!(
                "SELECT * FROM {} WHERE {} BETWEEN {} AND {}",
                tbl, col, low, high
            )
        })
}

/// Generate a SELECT with IS NULL
pub fn select_with_is_null() -> impl Strategy<Value = String> {
    (column_name(), table_name())
        .prop_map(|(col, tbl)| format!("SELECT * FROM {} WHERE {} IS NULL", tbl, col))
}

/// Generate a SELECT with IS NOT NULL
pub fn select_with_is_not_null() -> impl Strategy<Value = String> {
    (column_name(), table_name())
        .prop_map(|(col, tbl)| format!("SELECT * FROM {} WHERE {} IS NOT NULL", tbl, col))
}

/// Generate a CAST expression
pub fn cast_expression() -> impl Strategy<Value = String> {
    let types = prop_oneof![
        Just("INTEGER".to_string()),
        Just("VARCHAR(255)".to_string()),
        Just("DECIMAL(10,2)".to_string()),
        Just("TEXT".to_string()),
        Just("BOOLEAN".to_string()),
    ];

    (simple_expr(), types).prop_map(|(expr, typ)| format!("CAST({} AS {})", expr, typ))
}

/// Generate a SELECT with CAST
pub fn select_with_cast() -> impl Strategy<Value = String> {
    (cast_expression(), alias(), table_name())
        .prop_map(|(cast, a, tbl)| format!("SELECT {} AS {} FROM {}", cast, a, tbl))
}

/// Generate any valid PostgreSQL SELECT
pub fn any_select() -> impl Strategy<Value = String> {
    prop_oneof![
        simple_select(),
        select_with_group_by(),
        select_with_join(),
        select_distinct(),
        select_with_cte(),
        select_with_window(),
        select_union(),
        select_union_all(),
        select_with_case(),
        select_with_in_subquery(),
        select_with_exists(),
        select_with_between(),
        select_with_is_null(),
        select_with_is_not_null(),
        select_with_cast(),
    ]
}

// ===== PostgreSQL-specific syntax that smelt may not support =====
// These generators intentionally create SQL that may fail in smelt
// to help discover parser gaps.

/// Generate SQL with array subscript (known gap)
pub fn select_with_array_subscript() -> impl Strategy<Value = String> {
    (column_name(), table_name()).prop_map(|(col, tbl)| format!("SELECT {}[1] FROM {}", col, tbl))
}

/// Generate SQL with JSON operator (known gap)
pub fn select_with_json_operator() -> impl Strategy<Value = String> {
    (column_name(), table_name())
        .prop_map(|(col, tbl)| format!("SELECT {}->>'key' FROM {}", col, tbl))
}

/// Generate SQL with array literal (known gap)
pub fn select_with_array_literal() -> impl Strategy<Value = String> {
    table_name().prop_map(|tbl| format!("SELECT ARRAY[1, 2, 3] FROM {}", tbl))
}

/// Generate SQL with LIKE operator (known gap)
pub fn select_with_like() -> impl Strategy<Value = String> {
    (column_name(), table_name(), string_literal())
        .prop_map(|(col, tbl, pat)| format!("SELECT * FROM {} WHERE {} LIKE {}", tbl, col, pat))
}

/// Generate SQL with string concatenation (known gap)
pub fn select_with_concat() -> impl Strategy<Value = String> {
    (column_name(), column_name(), table_name())
        .prop_map(|(c1, c2, tbl)| format!("SELECT {} || ' ' || {} FROM {}", c1, c2, tbl))
}

/// Generate SQL known to trigger parser gaps
pub fn gap_triggering_select() -> impl Strategy<Value = String> {
    prop_oneof![
        select_with_array_subscript(),
        select_with_json_operator(),
        select_with_array_literal(),
        select_with_like(),
        select_with_concat(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    proptest! {
        #[test]
        fn test_identifier_valid(id in identifier()) {
            assert!(!id.is_empty());
            assert!(id.chars().next().unwrap().is_ascii_lowercase());
        }

        #[test]
        fn test_simple_select_structure(sql in simple_select()) {
            assert!(sql.starts_with("SELECT"));
            assert!(sql.contains("FROM"));
        }

        #[test]
        fn test_select_with_join_structure(sql in select_with_join()) {
            assert!(sql.contains("JOIN"));
        }

        #[test]
        fn test_select_with_cte_structure(sql in select_with_cte()) {
            assert!(sql.starts_with("WITH"));
        }

        #[test]
        fn test_select_with_window_structure(sql in select_with_window()) {
            assert!(sql.contains("OVER"));
        }
    }
}
