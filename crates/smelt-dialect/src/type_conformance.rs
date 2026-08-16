//! Type-conforming cast insertion.
//!
//! Wraps compiled SQL in a subquery that CASTs every SELECT column to the
//! smelt-inferred type.  This guarantees that backend output types match
//! smelt's type system exactly, regardless of backend-specific type rules.

use crate::SqlDialect;
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
/// The `dialect` controls how string types are emitted:
/// - DuckDB/PostgreSQL: `VARCHAR` (no length required)
/// - SparkSQL: `STRING` (`VARCHAR` without length is rejected by Spark 4+)
pub fn wrap_with_type_casts(
    sql: &str,
    column_names: &[&str],
    column_types: &[DataType],
    dialect: SqlDialect,
) -> String {
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
            DataType::Unknown(_) | DataType::Null => {
                select_items.push(name.to_string());
            }
            _ => {
                let type_sql = type_cast_sql(dt, dialect);
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

/// Returns the SQL type string to use in a CAST expression for the given dialect.
///
/// Spark 4+ requires `VARCHAR` to carry a length; use `STRING` for bare string casts.
fn type_cast_sql(dt: &DataType, dialect: SqlDialect) -> String {
    match (dt, dialect) {
        // Spark: VARCHAR without length → STRING
        (DataType::Text, SqlDialect::SparkSQL)
        | (DataType::Varchar { max_length: None }, SqlDialect::SparkSQL) => "STRING".to_string(),
        // GoogleSQL rejects the string and floating-point names `to_backend_sql`
        // emits: VARCHAR, TEXT, DOUBLE, REAL and FLOAT are each `Type not found`
        // (verified live — scripts/bigquery-probe4.sh, which also confirms the
        // integer aliases, DECIMAL, TIMESTAMP and DATE are accepted verbatim).
        // Only the rejected families are rewritten; everything else passes through.
        (dt, SqlDialect::BigQuery) => match dt {
            DataType::Text | DataType::Varchar { .. } | DataType::Char { .. } => {
                "STRING".to_string()
            }
            DataType::Float | DataType::Double => "FLOAT64".to_string(),
            DataType::Blob => "BYTES".to_string(),
            other => other.to_backend_sql(),
        },
        _ => dt.to_backend_sql(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_wrapping() {
        let sql = "SELECT 1 AS a, 2 AS b FROM t";
        let result = wrap_with_type_casts(
            sql,
            &["a", "b"],
            &[DataType::Integer, DataType::BigInt],
            SqlDialect::DuckDB,
        );
        assert_eq!(
            result,
            "SELECT CAST(a AS INTEGER) AS a, CAST(b AS BIGINT) AS b FROM (\n  SELECT 1 AS a, 2 AS b FROM t\n) _smelt_typed"
        );
    }

    #[test]
    fn text_becomes_varchar_for_duckdb() {
        let sql = "SELECT UPPER(x) AS u FROM t";
        let result = wrap_with_type_casts(sql, &["u"], &[DataType::Text], SqlDialect::DuckDB);
        assert!(result.contains("CAST(u AS VARCHAR) AS u"));
        assert!(!result.contains("TEXT"));
    }

    /// GoogleSQL has no VARCHAR, TEXT, DOUBLE, REAL or FLOAT — each is
    /// `Type not found` (verified live, scripts/bigquery-probe4.sh). The output
    /// cast wrap applies to every model on every backend, so an unmapped name
    /// here makes every BigQuery model fail at the boundary.
    #[test]
    fn rejected_type_names_are_mapped_for_bigquery() {
        let sql = "SELECT x AS s FROM t";
        for (dt, expected) in [
            (DataType::Text, "STRING"),
            (DataType::Varchar { max_length: None }, "STRING"),
            (
                DataType::Varchar {
                    max_length: Some(8),
                },
                "STRING",
            ),
            (DataType::Char { length: 3 }, "STRING"),
            (DataType::Double, "FLOAT64"),
            (DataType::Float, "FLOAT64"),
            (DataType::Blob, "BYTES"),
        ] {
            let result =
                wrap_with_type_casts(sql, &["s"], std::slice::from_ref(&dt), SqlDialect::BigQuery);
            assert!(
                result.contains(&format!("CAST(s AS {expected}) AS s")),
                "{dt:?} should cast to {expected} on BigQuery, got: {result}"
            );
        }
    }

    /// Names GoogleSQL accepts verbatim must not be rewritten.
    #[test]
    fn accepted_type_names_pass_through_for_bigquery() {
        let sql = "SELECT x AS s FROM t";
        for (dt, expected) in [
            (DataType::Integer, "INTEGER"),
            (DataType::BigInt, "BIGINT"),
            (DataType::Boolean, "BOOLEAN"),
            (DataType::Date, "DATE"),
        ] {
            let result =
                wrap_with_type_casts(sql, &["s"], std::slice::from_ref(&dt), SqlDialect::BigQuery);
            assert!(
                result.contains(&format!("CAST(s AS {expected}) AS s")),
                "{dt:?} should stay {expected} on BigQuery, got: {result}"
            );
        }
    }

    #[test]
    fn text_becomes_string_for_spark() {
        let sql = "SELECT UPPER(x) AS u FROM t";
        let result = wrap_with_type_casts(sql, &["u"], &[DataType::Text], SqlDialect::SparkSQL);
        assert!(result.contains("CAST(u AS STRING) AS u"), "got: {result}");
    }

    #[test]
    fn varchar_no_length_becomes_string_for_spark() {
        let sql = "SELECT x AS s FROM t";
        let result = wrap_with_type_casts(
            sql,
            &["s"],
            &[DataType::Varchar { max_length: None }],
            SqlDialect::SparkSQL,
        );
        assert!(result.contains("CAST(s AS STRING) AS s"), "got: {result}");
    }

    #[test]
    fn unknown_passes_through() {
        let sql = "SELECT x AS a, y AS b FROM t";
        let result = wrap_with_type_casts(
            sql,
            &["a", "b"],
            &[
                DataType::Unknown(smelt_types::UnknownReason::Dynamic),
                DataType::Integer,
            ],
            SqlDialect::DuckDB,
        );
        assert!(result.contains("a, CAST(b AS INTEGER) AS b"));
    }

    #[test]
    fn empty_columns_returns_original() {
        let sql = "SELECT * FROM t";
        let result = wrap_with_type_casts(sql, &[], &[], SqlDialect::DuckDB);
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
            SqlDialect::DuckDB,
        );
        assert!(result.contains("CAST(d AS DECIMAL(10,2)) AS d"));
    }
}
