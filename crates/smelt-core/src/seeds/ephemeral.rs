//! Ephemeral seed support: building inline row-set CTE bodies.
//!
//! When a seed is declared `materialization: ephemeral`, the compile-time path
//! replaces `smelt.<path>` references with a CTE whose body carries every row
//! of the seed's CSV with explicit per-column `CAST`. The row-set constructor
//! itself — `VALUES (…)` where the dialect supports one, `SELECT … UNION ALL
//! SELECT …` where it doesn't (GoogleSQL) — is owned by
//! [`smelt_core::sql::row_set`]; this module supplies the seed-specific
//! per-cell literal formatting and the CTE alias convention.
//!
//! References: seeds.md Semantics §7, Constraints & Invariants #7.
//!
//! CTE form (DuckDB/Spark):
//!
//! ```sql
//! __smelt_lookup_regions(region_id, region_name) AS (
//!   VALUES (CAST(1 AS INTEGER), CAST('North' AS VARCHAR)),
//!          (CAST(2 AS INTEGER), CAST('South' AS VARCHAR))
//! )
//! ```
//!
//! The CTE alias is `__smelt_{address_segments.join("_")}`.

use super::csv::CsvRecord;
use crate::config::BackendType;
use crate::sql::row_set::row_set_body;
use smelt_types::DataType;

/// Build the body of an inline row-set CTE for an ephemeral seed.
///
/// Returns the SQL string for the CTE body (everything between `AS (` and `)`).
/// The caller is responsible for wrapping it with the CTE alias. The row-set
/// constructor itself — `VALUES (…)` where `dialect` supports one, `SELECT …
/// UNION ALL SELECT …` where it doesn't (GoogleSQL) — comes from
/// [`row_set_body`]; this function's own job is turning each CSV cell into
/// the seed's typed `CAST(… AS <type>)` literal.
///
/// `columns` — `(column_name, DataType)` pairs in header order.
/// `rows` — CSV data rows.
///
/// # NULL handling
/// An empty/absent cell becomes `NULL` (cast to the declared type). A
/// wholly empty `rows` (the seed's CSV has a header but no data) emits a
/// single row of typed NULLs rather than reaching [`row_set_body`] with an
/// empty row set — [`row_set_body`] requires at least one row, and a
/// synthesized NULL row preserves the CTE's column names/types the same way
/// an empty row set would if one existed; callers filter it out downstream.
///
/// # Example output (DuckDB/Spark)
/// ```text
/// VALUES (CAST(1 AS INTEGER), CAST('North' AS VARCHAR)), (CAST(2 AS INTEGER), CAST('South' AS VARCHAR))
/// ```
pub fn build_values_cte(
    dialect: BackendType,
    columns: &[(String, DataType)],
    rows: &[CsvRecord],
) -> String {
    let col_names: Vec<&str> = columns.iter().map(|(n, _)| n.as_str()).collect();

    if rows.is_empty() {
        // No data — emit a single row of NULLs so the column names/types are
        // preserved in the CTE; callers can handle this edge case.
        let nulls: Vec<String> = columns
            .iter()
            .map(|(_, dt)| format!("CAST(NULL AS {})", datatype_to_sql(dt)))
            .collect();
        return row_set_body(dialect, &col_names, &[nulls]);
    }

    let row_parts: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .enumerate()
                .map(|(col_idx, (_, dt))| {
                    let raw = row.fields.get(col_idx).and_then(|v| v.as_deref());
                    match raw {
                        None => format!("CAST(NULL AS {})", datatype_to_sql(dt)),
                        Some(v) => {
                            format!("CAST({} AS {})", sql_literal(v, dt), datatype_to_sql(dt))
                        }
                    }
                })
                .collect()
        })
        .collect();

    row_set_body(dialect, &col_names, &row_parts)
}

/// Build the full CTE fragment `<alias>(<col1>, <col2>, ...) AS (<body>)`.
///
/// The alias follows the `__smelt_<address_segments.join("_")>` convention.
pub fn build_full_cte(
    dialect: BackendType,
    address_segments: &[String],
    columns: &[(String, DataType)],
    rows: &[CsvRecord],
) -> (String, String) {
    let alias = format!("__smelt_{}", address_segments.join("_"));
    let col_names: Vec<&str> = columns.iter().map(|(n, _)| n.as_str()).collect();
    let alias_with_cols = format!("{}({})", alias, col_names.join(", "));
    let body = build_values_cte(dialect, columns, rows);
    (alias_with_cols, body)
}

/// Format a CSV string value as a SQL literal for the given type.
fn sql_literal(v: &str, dt: &DataType) -> String {
    match dt {
        // Numeric types: emit the value directly (no quotes).
        DataType::Boolean | DataType::Integer | DataType::Decimal { .. } | DataType::Double => {
            v.to_string()
        }
        // All other types: single-quote the value, escaping internal quotes.
        _ => format!("'{}'", v.replace('\'', "''")),
    }
}

/// Map a `DataType` to the SQL type name used in `CAST(… AS <type>)`.
fn datatype_to_sql(dt: &DataType) -> &'static str {
    match dt {
        DataType::Boolean => "BOOLEAN",
        DataType::Integer => "INTEGER",
        DataType::Decimal { .. } => "DECIMAL",
        DataType::Double => "DOUBLE",
        DataType::Date => "DATE",
        DataType::Timestamp { .. } => "TIMESTAMP",
        // Text, Varchar, and all other types → VARCHAR
        _ => "VARCHAR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(idx: usize, fields: &[Option<&str>]) -> CsvRecord {
        CsvRecord {
            row_index: idx,
            fields: fields.iter().map(|f| f.map(|s| s.to_string())).collect(),
        }
    }

    #[test]
    fn values_cte_basic() {
        let columns = vec![
            ("id".to_string(), DataType::Integer),
            ("name".to_string(), DataType::Text),
        ];
        let rows = vec![
            row(1, &[Some("1"), Some("Alice")]),
            row(2, &[Some("2"), Some("Bob")]),
        ];
        let body = build_values_cte(BackendType::DuckDB, &columns, &rows);
        assert!(body.contains("VALUES"), "should contain VALUES");
        assert!(body.contains("CAST(1 AS INTEGER)"), "should cast integer");
        assert!(
            body.contains("CAST('Alice' AS VARCHAR)"),
            "should cast text"
        );
        assert!(body.contains("CAST('Bob' AS VARCHAR)"));
    }

    #[test]
    fn values_cte_null_cell() {
        let columns = vec![("x".to_string(), DataType::Integer)];
        let rows = vec![row(1, &[None])];
        let body = build_values_cte(BackendType::DuckDB, &columns, &rows);
        assert!(
            body.contains("CAST(NULL AS INTEGER)"),
            "null should become CAST(NULL AS INTEGER)"
        );
    }

    #[test]
    fn values_cte_empty_rows() {
        let columns = vec![("x".to_string(), DataType::Text)];
        let rows: Vec<CsvRecord> = vec![];
        let body = build_values_cte(BackendType::DuckDB, &columns, &rows);
        assert!(body.contains("VALUES"));
        assert!(body.contains("CAST(NULL AS VARCHAR)"));
    }

    #[test]
    fn full_cte_alias_format() {
        let segments = vec!["lookup".to_string(), "regions".to_string()];
        let columns = vec![("id".to_string(), DataType::Integer)];
        let rows = vec![row(1, &[Some("1")])];
        let (alias, body) = build_full_cte(BackendType::DuckDB, &segments, &columns, &rows);
        assert_eq!(alias, "__smelt_lookup_regions(id)");
        assert!(body.contains("VALUES"));
    }

    /// The rejection test at the `ephemeral` call site: for
    /// `BackendType::BigQuery` the emitted CTE body is never a `VALUES (…)`
    /// table-value constructor, and for DuckDB/Spark it routes through
    /// [`row_set_body`] rather than formatting its own — both dialects
    /// agree on the shared owner's `VALUES` rendering.
    #[test]
    fn values_cte_bigquery_is_not_a_values_constructor() {
        let columns = vec![
            ("id".to_string(), DataType::Integer),
            ("name".to_string(), DataType::Text),
        ];
        let rows = vec![row(1, &[Some("1"), Some("North")])];
        let body = build_values_cte(BackendType::BigQuery, &columns, &rows);
        assert!(
            !body.contains("VALUES"),
            "BigQuery has no table-value constructor, got: {body}"
        );
        assert!(body.contains("UNION ALL") || !body.contains(", ("));
        assert!(body.contains("CAST(1 AS INTEGER) AS id"));
    }

    #[test]
    fn values_cte_duckdb_and_spark_agree_via_the_shared_owner() {
        let columns = vec![("id".to_string(), DataType::Integer)];
        let rows = vec![row(1, &[Some("1")])];
        let duckdb = build_values_cte(BackendType::DuckDB, &columns, &rows);
        let spark = build_values_cte(BackendType::Spark, &columns, &rows);
        assert_eq!(duckdb, spark);
        assert_eq!(
            duckdb,
            row_set_body(
                BackendType::DuckDB,
                &["id"],
                &[vec!["CAST(1 AS INTEGER)".to_string()]]
            )
        );
    }

    #[test]
    fn sql_literal_escapes_single_quotes() {
        let result = sql_literal("O'Brien", &DataType::Text);
        assert_eq!(result, "'O''Brien'");
    }
}
