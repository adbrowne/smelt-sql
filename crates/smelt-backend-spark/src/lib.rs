//! Spark backend implementation for smelt via PySpark/PyO3 bridge.
//!
//! Uses PyO3 to call PySpark for SQL execution and Arrow result conversion.
//! All SQL generation and orchestration stays in Rust; Python handles only
//! Spark session management and query execution.
//!
//! Works with:
//! - Local Spark (via Spark Connect: `sc://localhost:15002`)
//! - Databricks Connect (`pip install databricks-connect`)
//! - Any PySpark-compatible environment

use arrow::array::RecordBatch;
use arrow::pyarrow::FromPyArrow;
use async_trait::async_trait;
use pyo3::prelude::*;
use smelt_backend::{Backend, BackendCapabilities, BackendError, PartitionSpec, SqlDialect};

/// Spark backend for smelt, powered by PySpark via PyO3.
///
/// Holds a Python `SparkAdapter` object that wraps a PySpark SparkSession.
/// All Python calls go through `spawn_blocking` to avoid blocking the async runtime.
pub struct SparkBackend {
    adapter: Py<PyAny>,
    catalog: String,
    #[allow(dead_code)]
    schema: String,
}

// Safety: Py<PyAny> is Send, and we only access it inside `Python::attach`
// which acquires the GIL. All access is serialized through spawn_blocking.
unsafe impl Send for SparkBackend {}
unsafe impl Sync for SparkBackend {}

impl SparkBackend {
    /// Create a new Spark backend by initializing a PySpark session.
    ///
    /// # Arguments
    /// * `connect_url` - Spark Connect URL (e.g., "sc://localhost:15002")
    /// * `catalog` - Catalog name (e.g., "spark_catalog")
    /// * `schema` - Schema name (e.g., "default")
    pub async fn new(connect_url: &str, catalog: &str, schema: &str) -> Result<Self, BackendError> {
        let connect_url = connect_url.to_string();
        let catalog = catalog.to_string();
        let schema = schema.to_string();

        let adapter = tokio::task::spawn_blocking({
            let catalog = catalog.clone();
            let schema = schema.clone();
            move || {
                Python::attach(|py| {
                    let module = py.import("smelt.spark_adapter").map_err(|e| {
                        BackendError::connection_failed(format!(
                            "Failed to import smelt.spark_adapter: {}. \
                             Ensure PySpark is installed: pip install pyspark",
                            e
                        ))
                    })?;

                    let cls = module.getattr("SparkAdapter").map_err(|e| {
                        BackendError::connection_failed(format!(
                            "SparkAdapter class not found: {}",
                            e
                        ))
                    })?;

                    let adapter = cls.call1((&connect_url, &catalog, &schema)).map_err(|e| {
                        BackendError::connection_failed(format!(
                            "Failed to create SparkSession: {}",
                            e
                        ))
                    })?;

                    Ok::<Py<PyAny>, BackendError>(adapter.unbind())
                })
            }
        })
        .await
        .map_err(|e| BackendError::Other(anyhow::anyhow!("spawn_blocking join error: {}", e)))??;

        tracing::info!(
            "Spark session established (catalog={}, schema={})",
            catalog,
            schema
        );

        Ok(Self {
            adapter,
            catalog,
            schema,
        })
    }

    /// Build a fully qualified table name: catalog.schema.table
    fn qualified_name(&self, schema: &str, name: &str) -> String {
        format!("{}.{}.{}", self.catalog, schema, name)
    }

    /// Execute SQL via the Python adapter, returning Arrow RecordBatches.
    async fn py_execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        let sql = sql.to_string();

        // Clone the adapter reference inside GIL
        let adapter = Python::attach(|py| self.adapter.clone_ref(py));

        tokio::task::spawn_blocking(move || {
            Python::attach(|py| {
                let result = adapter
                    .call_method1(py, "execute_sql", (&sql,))
                    .map_err(|e| BackendError::execution_failed("spark sql", format!("{}", e)))?;

                // Convert pyarrow.Table to Vec<RecordBatch>
                let table = result.bind(py);
                let batches_py = table.call_method0("to_batches").map_err(|e| {
                    BackendError::execution_failed(
                        "arrow conversion",
                        format!("Failed to convert to batches: {}", e),
                    )
                })?;

                let mut batches = Vec::new();
                let iter = batches_py.try_iter().map_err(|e| {
                    BackendError::execution_failed(
                        "arrow conversion",
                        format!("Failed to iterate batches: {}", e),
                    )
                })?;
                for batch_result in iter {
                    let batch_py: Bound<'_, PyAny> = batch_result.map_err(|e| {
                        BackendError::execution_failed(
                            "arrow conversion",
                            format!("Failed to get batch: {}", e),
                        )
                    })?;
                    let batch = RecordBatch::from_pyarrow_bound(&batch_py).map_err(|e| {
                        BackendError::execution_failed(
                            "arrow conversion",
                            format!("Failed to convert RecordBatch: {}", e),
                        )
                    })?;
                    batches.push(batch);
                }

                Ok(batches)
            })
        })
        .await
        .map_err(|e| BackendError::Other(anyhow::anyhow!("spawn_blocking join error: {}", e)))?
    }

    /// Execute SQL without collecting results (for DDL/DML statements).
    async fn py_execute_no_result(&self, sql: &str) -> Result<(), BackendError> {
        let sql = sql.to_string();
        let adapter = Python::attach(|py| self.adapter.clone_ref(py));

        tokio::task::spawn_blocking(move || {
            Python::attach(|py| {
                adapter
                    .call_method1(py, "execute_sql_no_result", (&sql,))
                    .map_err(|e| BackendError::execution_failed("spark sql", format!("{}", e)))?;
                Ok(())
            })
        })
        .await
        .map_err(|e| BackendError::Other(anyhow::anyhow!("spawn_blocking join error: {}", e)))?
    }
}

#[async_trait]
impl Backend for SparkBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        tracing::debug!("Spark execute_sql: {}", truncate_sql(sql));
        self.py_execute_sql(sql).await
    }

    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, name);
        tracing::debug!("Spark CREATE TABLE {} AS ...", table_name);

        // Spark doesn't reliably support CREATE OR REPLACE TABLE,
        // so we DROP IF EXISTS first, then CREATE TABLE ... AS SELECT
        let drop_sql = format!("DROP TABLE IF EXISTS {}", table_name);
        self.py_execute_no_result(&drop_sql).await?;

        let create_sql = format!("CREATE TABLE {} AS {}", table_name, sql);
        self.py_execute_no_result(&create_sql).await
    }

    async fn create_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let view_name = self.qualified_name(schema, name);
        tracing::debug!("Spark CREATE OR REPLACE VIEW {} AS ...", view_name);

        let create_sql = format!("CREATE OR REPLACE VIEW {} AS {}", view_name, sql);
        self.py_execute_no_result(&create_sql).await
    }

    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, name);
        let sql = format!("DROP TABLE IF EXISTS {}", table_name);
        self.py_execute_no_result(&sql).await
    }

    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        let view_name = self.qualified_name(schema, name);
        let sql = format!("DROP VIEW IF EXISTS {}", view_name);
        self.py_execute_no_result(&sql).await
    }

    async fn get_row_count(&self, schema: &str, name: &str) -> Result<usize, BackendError> {
        let table_name = self.qualified_name(schema, name);
        let adapter = Python::attach(|py| self.adapter.clone_ref(py));

        tokio::task::spawn_blocking(move || {
            Python::attach(|py| {
                let count: usize = adapter
                    .call_method1(py, "get_row_count", (&table_name,))
                    .map_err(|e| {
                        BackendError::execution_failed(
                            table_name.clone(),
                            format!("Failed to get row count: {}", e),
                        )
                    })?
                    .extract(py)
                    .map_err(|e| {
                        BackendError::execution_failed(
                            table_name,
                            format!("Failed to extract row count: {}", e),
                        )
                    })?;
                Ok(count)
            })
        })
        .await
        .map_err(|e| BackendError::Other(anyhow::anyhow!("spawn_blocking join error: {}", e)))?
    }

    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, BackendError> {
        let table_name = self.qualified_name(schema, name);
        let sql = format!("SELECT * FROM {} LIMIT {}", table_name, limit);
        self.py_execute_sql(&sql).await
    }

    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        let table_name = self.qualified_name(schema, name);
        let adapter = Python::attach(|py| self.adapter.clone_ref(py));

        tokio::task::spawn_blocking(move || {
            Python::attach(|py| {
                let exists: bool = adapter
                    .call_method1(py, "table_exists", (&table_name,))
                    .map_err(|e| {
                        BackendError::execution_failed(
                            table_name.clone(),
                            format!("Failed to check table existence: {}", e),
                        )
                    })?
                    .extract(py)
                    .map_err(|e| {
                        BackendError::execution_failed(
                            table_name,
                            format!("Failed to extract boolean: {}", e),
                        )
                    })?;
                Ok(exists)
            })
        })
        .await
        .map_err(|e| BackendError::Other(anyhow::anyhow!("spawn_blocking join error: {}", e)))?
    }

    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        let sql = format!("CREATE DATABASE IF NOT EXISTS {}.{}", self.catalog, schema);
        self.py_execute_no_result(&sql).await
    }

    fn dialect(&self) -> SqlDialect {
        SqlDialect::SparkSQL
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::spark()
    }

    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionSpec,
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, name);

        let values_list = partition
            .values
            .iter()
            .map(|v| format!("'{}'", v.replace("'", "''")))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "DELETE FROM {} WHERE {} IN ({})",
            table_name, partition.column, values_list
        );

        self.py_execute_no_result(&sql).await
    }

    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, name);
        let insert_sql = format!("INSERT INTO {} {}", table_name, sql);
        self.py_execute_no_result(&insert_sql).await
    }

    async fn merge_into(
        &self,
        schema: &str,
        table: &str,
        source_sql: &str,
        unique_key: &[String],
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, table);

        let on_clause = unique_key
            .iter()
            .map(|k| format!("target.{} = source.{}", k, k))
            .collect::<Vec<_>>()
            .join(" AND ");

        let merge_sql = format!(
            "MERGE INTO {} AS target USING ({}) AS source ON {} \
             WHEN MATCHED THEN UPDATE SET * \
             WHEN NOT MATCHED THEN INSERT *",
            table_name, source_sql, on_clause
        );

        self.py_execute_no_result(&merge_sql).await
    }

    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionSpec,
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, table);

        // Spark has native INSERT OVERWRITE support
        let insert_sql = format!(
            "INSERT OVERWRITE TABLE {} PARTITION ({}) {}",
            table_name, partition.column, sql
        );

        self.py_execute_no_result(&insert_sql).await
    }
}

/// Truncate SQL for logging (first 200 chars).
fn truncate_sql(sql: &str) -> String {
    if sql.len() > 200 {
        format!("{}...", &sql[..200])
    } else {
        sql.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qualified_name() {
        let catalog = "spark_catalog";
        let schema = "default";
        let name = "my_table";
        let qualified = format!("{}.{}.{}", catalog, schema, name);
        assert_eq!(qualified, "spark_catalog.default.my_table");
    }

    #[test]
    fn test_truncate_sql() {
        assert_eq!(truncate_sql("SELECT 1"), "SELECT 1");

        let long_sql = "x".repeat(300);
        let truncated = truncate_sql(&long_sql);
        assert!(truncated.len() < 210);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn test_partition_values_escaping() {
        let partition = PartitionSpec {
            column: "dt".to_string(),
            values: vec!["2024-01-01".to_string(), "it's".to_string()],
        };

        let values_list = partition
            .values
            .iter()
            .map(|v| format!("'{}'", v.replace("'", "''")))
            .collect::<Vec<_>>()
            .join(", ");

        assert_eq!(values_list, "'2024-01-01', 'it''s'");
    }

    #[test]
    fn test_merge_on_clause() {
        let unique_key = ["id".to_string(), "date".to_string()];
        let on_clause = unique_key
            .iter()
            .map(|k| format!("target.{} = source.{}", k, k))
            .collect::<Vec<_>>()
            .join(" AND ");

        assert_eq!(
            on_clause,
            "target.id = source.id AND target.date = source.date"
        );
    }
}
