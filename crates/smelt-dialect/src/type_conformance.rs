//! Type-conforming cast insertion.
//!
//! Wraps compiled SQL in a subquery that CASTs every SELECT column to the
//! smelt-inferred type.  This guarantees that backend output types match
//! smelt's type system exactly, regardless of backend-specific type rules.

use smelt_types::DataType;

/// Wrap `sql` in a subquery that CASTs each column to its smelt-inferred type.
///
/// Produces:
/// ```sql
/// SELECT CAST(c1 AS T1) AS c1, CAST(c2 AS T2) AS c2
/// FROM (
///   <original sql>
/// ) _smelt_typed
/// ```
///
/// Columns with `Unknown` or `Null` types are passed through without casting.
pub fn wrap_with_type_casts(sql: &str, column_names: &[&str], column_types: &[DataType]) -> String {
    assert_eq!(
        column_names.len(),
        column_types.len(),
        "column_names and column_types must have the same length"
    );

    if column_names.is_empty() {
        return sql.to_string();
    }

    let mut select_items = Vec::with_capacity(column_names.len());
    for (name, dt) in column_names.iter().zip(column_types.iter()) {
        match dt {
            DataType::Unknown | DataType::Null => {
                select_items.push(name.to_string());
            }
            _ => {
                let type_sql = dt.to_backend_sql();
                select_items.push(format!("CAST({name} AS {type_sql}) AS {name}"));
            }
        }
    }

    format!(
        "SELECT {} FROM (\n  {}\n) _smelt_typed",
        select_items.join(", "),
        sql
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_wrapping() {
        let sql = "SELECT 1 AS a, 2 AS b FROM t";
        let result = wrap_with_type_casts(sql, &["a", "b"], &[DataType::Integer, DataType::BigInt]);
        assert_eq!(
            result,
            "SELECT CAST(a AS INTEGER) AS a, CAST(b AS BIGINT) AS b FROM (\n  SELECT 1 AS a, 2 AS b FROM t\n) _smelt_typed"
        );
    }

    #[test]
    fn text_becomes_varchar() {
        let sql = "SELECT UPPER(x) AS u FROM t";
        let result = wrap_with_type_casts(sql, &["u"], &[DataType::Text]);
        assert!(result.contains("CAST(u AS VARCHAR) AS u"));
        assert!(!result.contains("TEXT"));
    }

    #[test]
    fn unknown_passes_through() {
        let sql = "SELECT x AS a, y AS b FROM t";
        let result =
            wrap_with_type_casts(sql, &["a", "b"], &[DataType::Unknown, DataType::Integer]);
        assert!(result.contains("a, CAST(b AS INTEGER) AS b"));
    }

    #[test]
    fn empty_columns_returns_original() {
        let sql = "SELECT * FROM t";
        let result = wrap_with_type_casts(sql, &[], &[]);
        assert_eq!(result, sql);
    }

    #[test]
    fn decimal_with_precision() {
        let sql = "SELECT x AS d FROM t";
        let result = wrap_with_type_casts(
            sql,
            &["d"],
            &[DataType::Decimal {
                precision: 10,
                scale: 2,
            }],
        );
        assert!(result.contains("CAST(d AS DECIMAL(10,2)) AS d"));
    }
}
