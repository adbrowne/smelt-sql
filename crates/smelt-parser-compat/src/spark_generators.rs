//! Spark SQL-specific generators for property-based testing
//!
//! Generates SQL that uses Spark-specific syntax (QUALIFY, lambda expressions,
//! PIVOT/UNPIVOT, array subscript/slice) for testing against sqlparser-rs
//! DatabricksDialect and sqlglot.

use proptest::prelude::*;

use crate::pg_generators::{
    alias, column_name, comparison_expr, numeric_literal, select_list, table_name, where_clause,
};

/// Generate a SELECT with QUALIFY clause
pub fn select_with_qualify() -> impl Strategy<Value = String> {
    (select_list(), table_name(), column_name(), where_clause()).prop_map(
        |(sel, tbl, order_col, whr)| {
            let mut sql = format!("SELECT {} FROM {}", sel, tbl);
            if let Some(w) = whr {
                sql.push_str(&format!(" {}", w));
            }
            sql.push_str(&format!(
                " QUALIFY ROW_NUMBER() OVER (ORDER BY {}) = 1",
                order_col
            ));
            sql
        },
    )
}

/// Generate a SELECT with QUALIFY using PARTITION BY
pub fn select_with_qualify_partition() -> impl Strategy<Value = String> {
    (column_name(), column_name(), table_name()).prop_map(|(part_col, order_col, tbl)| {
        format!(
            "SELECT * FROM {} QUALIFY ROW_NUMBER() OVER (PARTITION BY {} ORDER BY {}) = 1",
            tbl, part_col, order_col
        )
    })
}

/// Generate a TRANSFORM lambda expression
pub fn select_with_transform() -> impl Strategy<Value = String> {
    (column_name(), table_name()).prop_map(|(arr_col, tbl)| {
        format!("SELECT TRANSFORM({}, x -> x + 1) FROM {}", arr_col, tbl)
    })
}

/// Generate an AGGREGATE lambda expression
pub fn select_with_aggregate_func() -> impl Strategy<Value = String> {
    (column_name(), table_name()).prop_map(|(arr_col, tbl)| {
        format!(
            "SELECT AGGREGATE({}, 0, (acc, x) -> acc + x) FROM {}",
            arr_col, tbl
        )
    })
}

/// Generate a SELECT with PIVOT
pub fn select_with_pivot() -> impl Strategy<Value = String> {
    (table_name(), column_name(), column_name()).prop_map(|(tbl, agg_col, pivot_col)| {
        format!(
            "SELECT * FROM {} PIVOT (SUM({}) FOR {} IN ('a', 'b', 'c'))",
            tbl, agg_col, pivot_col
        )
    })
}

/// Generate a SELECT with UNPIVOT
pub fn select_with_unpivot() -> impl Strategy<Value = String> {
    (table_name(), column_name(), column_name()).prop_map(|(tbl, col1, col2)| {
        format!(
            "SELECT * FROM {} UNPIVOT (val FOR name IN ({}, {}))",
            tbl, col1, col2
        )
    })
}

/// Generate a SELECT with array subscript
pub fn select_with_array_subscript() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple subscript: arr[1]
        (column_name(), table_name())
            .prop_map(|(col, tbl)| format!("SELECT {}[1] FROM {}", col, tbl)),
        // Subscript with expression index
        (column_name(), table_name(), numeric_literal())
            .prop_map(|(col, tbl, idx)| format!("SELECT {}[{}] FROM {}", col, tbl, idx)),
    ]
}

/// Generate a SELECT with array slice
pub fn select_with_array_slice() -> impl Strategy<Value = String> {
    (column_name(), table_name()).prop_map(|(col, tbl)| format!("SELECT {}[1:5] FROM {}", col, tbl))
}

/// Generate a SELECT with FILTER clause on aggregate
pub fn select_with_filter_aggregate() -> impl Strategy<Value = String> {
    (column_name(), table_name(), comparison_expr()).prop_map(|(col, tbl, cond)| {
        format!("SELECT COUNT({}) FILTER (WHERE {}) FROM {}", col, cond, tbl)
    })
}

/// Generate a SELECT with multiple Spark features combined
pub fn select_with_spark_window_qualify() -> impl Strategy<Value = String> {
    (column_name(), column_name(), table_name(), alias()).prop_map(
        |(sel_col, order_col, tbl, win_alias)| {
            format!(
                "SELECT {}, ROW_NUMBER() OVER (ORDER BY {}) AS {} FROM {} QUALIFY {} = 1",
                sel_col, order_col, win_alias, tbl, win_alias
            )
        },
    )
}

/// Generate any Spark-specific SELECT
pub fn any_spark_select() -> impl Strategy<Value = String> {
    prop_oneof![
        select_with_qualify(),
        select_with_qualify_partition(),
        select_with_transform(),
        select_with_aggregate_func(),
        select_with_pivot(),
        select_with_unpivot(),
        select_with_array_subscript(),
        select_with_array_slice(),
    ]
}

/// Generate SQL that should be valid in both standard SQL and Spark
/// (shared generators that work for both pg_query and sqlparser-databricks)
pub fn standard_sql_select() -> impl Strategy<Value = String> {
    // Re-export the standard generators that should work across all dialects
    prop_oneof![
        crate::pg_generators::simple_select(),
        crate::pg_generators::select_with_group_by(),
        crate::pg_generators::select_with_join(),
        crate::pg_generators::select_distinct(),
        crate::pg_generators::select_with_cte(),
        crate::pg_generators::select_with_window(),
        crate::pg_generators::select_union(),
        crate::pg_generators::select_union_all(),
        crate::pg_generators::select_with_case(),
        crate::pg_generators::select_with_in_subquery(),
        crate::pg_generators::select_with_exists(),
        crate::pg_generators::select_with_between(),
        crate::pg_generators::select_with_is_null(),
        crate::pg_generators::select_with_is_not_null(),
        crate::pg_generators::select_with_cast(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    proptest! {
        #[test]
        fn test_qualify_structure(sql in select_with_qualify()) {
            assert!(sql.contains("QUALIFY"));
            assert!(sql.contains("ROW_NUMBER"));
        }

        #[test]
        fn test_transform_structure(sql in select_with_transform()) {
            assert!(sql.contains("TRANSFORM"));
            assert!(sql.contains("->"));
        }

        #[test]
        fn test_pivot_structure(sql in select_with_pivot()) {
            assert!(sql.contains("PIVOT"));
            assert!(sql.contains("SUM"));
        }

        #[test]
        fn test_unpivot_structure(sql in select_with_unpivot()) {
            assert!(sql.contains("UNPIVOT"));
        }

        #[test]
        fn test_array_subscript_structure(sql in select_with_array_subscript()) {
            assert!(sql.contains("["));
        }
    }
}
