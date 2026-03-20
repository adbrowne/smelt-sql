//! DuckDB type oracle — executes SQL against DuckDB and extracts result column types.
//!
//! The `TypeOracle` trait enables future PostgreSQL/Spark backends without changing
//! the property test harness.

use super::arrow_mapping::arrow_to_smelt;
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

impl DuckDbOracle {
    pub fn new() -> Self {
        Self {
            conn: Connection::open_in_memory().expect("failed to open in-memory DuckDB"),
        }
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
}
