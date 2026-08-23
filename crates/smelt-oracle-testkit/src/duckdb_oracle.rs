//! DuckDB type oracle — executes SQL against DuckDB and extracts result column types.
//!
//! The `TypeOracle` trait enables future PostgreSQL/Spark backends without changing
//! the property test harness.

use crate::arrow_mapping::arrow_to_smelt;
use duckdb::Connection;
use smelt_types::DataType;

/// Backend-agnostic interface for querying the actual column types of a SQL statement.
pub trait TypeOracle {
    /// Execute `sql` and return `(column_name, DataType)` pairs from the result schema.
    fn query_types(&self, sql: &str) -> Result<Vec<(String, DataType)>, String>;
}

/// DuckDB-backed oracle using an in-memory database.
pub struct DuckDbOracle {
    conn: Connection,
}

impl Default for DuckDbOracle {
    fn default() -> Self {
        Self::new()
    }
}

impl DuckDbOracle {
    /// Open a fresh in-memory DuckDB.
    pub fn new() -> Self {
        Self {
            conn: Connection::open_in_memory().expect("failed to open in-memory DuckDB"),
        }
    }

    /// Execute one or more DDL/DML statements (separated by `;`).
    ///
    /// Used to set up tables with real NULL-bearing data for value-based nullability tests.
    pub fn execute_ddl(&self, sql: &str) -> Result<(), String> {
        // Split on `;` to handle multi-statement setup strings
        for stmt in sql.split(';') {
            let trimmed = stmt.trim();
            if trimmed.is_empty() {
                continue;
            }
            self.conn
                .execute_batch(trimmed)
                .map_err(|e| format!("DDL execute error: {e}\n  statement: {trimmed}"))?;
        }
        Ok(())
    }

    /// Execute a SELECT query and return the null count per column.
    ///
    /// Returns `Vec<(column_name, null_count)>` in column order.
    /// This is a value-based check: it scans actual result rows, not just the schema.
    ///
    /// The Arrow `null_count()` on each column array gives the exact count efficiently.
    pub fn count_nulls_per_column(&self, sql: &str) -> Result<Vec<(String, usize)>, String> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| format!("prepare: {e}"))?;
        let arrow_result = stmt.query_arrow([]).map_err(|e| format!("query: {e}"))?;

        let batches: Vec<_> = arrow_result.collect();
        if batches.is_empty() {
            return Err("query returned no batches".into());
        }

        // Accumulate null counts across all batches
        let schema = batches[0].schema();
        let num_cols = schema.fields().len();
        let mut null_counts: Vec<usize> = vec![0; num_cols];

        for batch in &batches {
            for (col_idx, array) in batch.columns().iter().enumerate() {
                // Arrow array `null_count()` is O(1) — computed at batch creation time.
                null_counts[col_idx] += array.null_count();
            }
        }

        let names: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();

        Ok(names.into_iter().zip(null_counts).collect())
    }
}

impl DuckDbOracle {
    /// Execute a query and return rows as sorted `Vec<Vec<String>>` for comparison.
    ///
    /// Each inner `Vec<String>` is one row's values as strings (in column order).
    /// The outer vec is sorted so that comparison is order-independent.
    pub fn execute_query(&self, sql: &str) -> Result<Vec<Vec<String>>, String> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| format!("prepare: {e}"))?;

        let mut rows_raw: Vec<Vec<String>> = Vec::new();

        let mut query_rows = stmt.query([]).map_err(|e| format!("query: {e}"))?;

        while let Some(row) = query_rows.next().map_err(|e| format!("row: {e}"))? {
            // Collect column values; grow the vec as we encounter each index.
            let mut row_vals: Vec<String> = Vec::new();
            let mut col_idx = 0usize;
            while let Ok(val) = row.get::<_, duckdb::types::Value>(col_idx) {
                row_vals.push(format!("{val:?}"));
                col_idx += 1;
            }
            rows_raw.push(row_vals);
        }

        // Sort rows for deterministic comparison (result sets are unordered).
        rows_raw.sort();
        Ok(rows_raw)
    }
}

impl TypeOracle for DuckDbOracle {
    fn query_types(&self, sql: &str) -> Result<Vec<(String, DataType)>, String> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| format!("prepare: {e}"))?;
        let arrow_result = stmt.query_arrow([]).map_err(|e| format!("query: {e}"))?;

        // We only need the schema, but DuckDB's Arrow interface requires collecting at least
        // one batch to materialize the schema.  The CTE queries are trivially small.
        let batches: Vec<_> = arrow_result.collect();

        let schema = if let Some(batch) = batches.first() {
            batch.schema()
        } else {
            return Err("query returned no batches".into());
        };

        let mut result = Vec::new();
        for field in schema.fields() {
            let smelt_type = arrow_to_smelt(field.data_type());
            result.push((field.name().clone(), smelt_type));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_test_integer() {
        let oracle = DuckDbOracle::new();
        let types = oracle
            .query_types("SELECT CAST(1 AS INTEGER) AS x")
            .unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].0, "x");
        assert_eq!(types[0].1, DataType::Integer);
    }

    #[test]
    fn smoke_test_multiple_columns() {
        let oracle = DuckDbOracle::new();
        let types = oracle
            .query_types("SELECT CAST(1 AS INTEGER) AS a, CAST('hi' AS VARCHAR) AS b")
            .unwrap();
        assert_eq!(types.len(), 2);
        assert_eq!(types[0].1, DataType::Integer);
        assert_eq!(types[1].1, DataType::Varchar { max_length: None });
    }

    #[test]
    fn json_type_check() {
        let oracle = DuckDbOracle::new();
        // DuckDB JSON maps to Varchar via Arrow
        let types = oracle
            .query_types("SELECT json_object('a', 1) AS j")
            .unwrap();
        assert_eq!(types[0].1, DataType::Varchar { max_length: None });
    }
}
