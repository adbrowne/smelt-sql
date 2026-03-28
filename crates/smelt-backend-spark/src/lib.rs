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

mod sql;

#[cfg(test)]
mod tests;

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
        sql::qualified_name(&self.catalog, schema, name)
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
        tracing::debug!("Spark execute_sql: {}", sql::truncate_sql(sql));
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
        self.py_execute_no_result(&sql::drop_table(&table_name))
            .await?;
        self.py_execute_no_result(&sql::create_table_as(&table_name, sql))
            .await
    }

    async fn create_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let view_name = self.qualified_name(schema, name);
        tracing::debug!("Spark CREATE OR REPLACE VIEW {} AS ...", view_name);
        self.py_execute_no_result(&sql::create_view_as(&view_name, sql))
            .await
    }

    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, name);
        self.py_execute_no_result(&sql::drop_table(&table_name))
            .await
    }

    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        let view_name = self.qualified_name(schema, name);
        self.py_execute_no_result(&sql::drop_view(&view_name)).await
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
        let sql = sql::select_preview(&table_name, limit);
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
        self.py_execute_no_result(&sql::create_database(&self.catalog, schema))
            .await
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
        self.py_execute_no_result(&sql::delete_partitions(&table_name, partition))
            .await
    }

    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, name);
        self.py_execute_no_result(&sql::insert_into(&table_name, sql))
            .await
    }

    async fn merge_into(
        &self,
        schema: &str,
        table: &str,
        source_sql: &str,
        unique_key: &[String],
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, table);
        self.py_execute_no_result(&sql::merge_into(&table_name, source_sql, unique_key))
            .await
    }

    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionSpec,
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, table);
        self.py_execute_no_result(&sql::insert_overwrite(&table_name, sql, partition))
            .await
    }
}
