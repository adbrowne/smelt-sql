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
use arrow::datatypes::SchemaRef;
use arrow::pyarrow::FromPyArrow;
use async_trait::async_trait;
use pyo3::prelude::*;
use smelt_backend::{
    emit_delete_insert, Backend, BackendCapabilities, BackendError, MaintenanceDialect,
    PartitionRange, Region, SqlDialect,
};

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
    /// Base directory for Parquet output (from target config `warehouse` field).
    warehouse: Option<String>,
    /// Whether to create managed tables as Delta (`USING DELTA`) or plain Parquet.
    /// Delta is required for DELETE / MERGE / schema-evolution operations.
    /// Parquet is used for the cross-engine read path (DuckDB reads raw Parquet).
    use_delta: bool,
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
    /// * `warehouse` - Optional base directory for Parquet output
    /// * `use_delta` - If true, managed tables are created with `USING DELTA`
    ///   (required for DELETE / MERGE / schema-evolution; default `true`).
    ///   Set to `false` for the cross-engine Parquet path where DuckDB reads
    ///   raw Parquet files via `read_parquet(...)`.
    pub async fn new(
        connect_url: &str,
        catalog: &str,
        schema: &str,
        warehouse: Option<&str>,
        use_delta: bool,
    ) -> Result<Self, BackendError> {
        let connect_url = connect_url.to_string();
        let catalog = catalog.to_string();
        let schema = schema.to_string();

        let adapter = tokio::task::spawn_blocking({
            let catalog = catalog.clone();
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

                    // Pass only connect_url + catalog — schema selection is deferred until
                    // after ensure_schema() creates the database (see below).
                    let adapter = cls.call1((&connect_url, &catalog)).map_err(|e| {
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

        let backend = Self {
            adapter,
            catalog,
            schema: schema.clone(),
            warehouse: warehouse.map(|s| s.to_string()),
            use_delta,
        };

        // Create the schema before selecting it (spec: multi_backend.md §Semantics
        // "Session initialization" — requires_schema_init = true for all backends).
        // ensure_schema() is the single source of the CREATE DATABASE statement;
        // select_current_schema() then makes it the session default.
        if !schema.is_empty() {
            backend.ensure_schema(&schema).await?;
            backend.py_select_schema(&schema).await?;
        }

        tracing::info!(
            "Spark session established (catalog={}, schema={})",
            backend.catalog,
            backend.schema
        );

        Ok(backend)
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

    /// Call Python `select_current_schema(schema)` to set the session's current database.
    ///
    /// Must only be called after `ensure_schema()` has created the schema — Spark's
    /// `setCurrentDatabase` raises `[SCHEMA_NOT_FOUND]` on a non-existent schema.
    async fn py_select_schema(&self, schema: &str) -> Result<(), BackendError> {
        let schema = schema.to_string();
        let adapter = Python::attach(|py| self.adapter.clone_ref(py));
        tokio::task::spawn_blocking(move || {
            Python::attach(|py| {
                adapter
                    .call_method1(py, "select_current_schema", (&schema,))
                    .map_err(|e| {
                        BackendError::connection_failed(format!(
                            "Failed to select schema '{}': {}",
                            schema, e
                        ))
                    })?;
                Ok(())
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
        // so we DROP IF EXISTS first, then CREATE TABLE ... AS SELECT.
        // If the previous run left a VIEW at this name (not a TABLE), Spark's
        // DROP TABLE rejects it with [WRONG_COMMAND_FOR_OBJECT_TYPE]; fall
        // back to DROP VIEW in that case so the CREATE TABLE can proceed.
        let drop_result = self
            .py_execute_no_result(&sql::drop_table(&table_name))
            .await;
        if let Err(ref e) = drop_result {
            let msg = e.to_string();
            if msg.contains("WRONG_COMMAND_FOR_OBJECT_TYPE") || msg.contains("is a VIEW") {
                tracing::debug!(
                    "DROP TABLE failed (object is a VIEW): {} — retrying with DROP VIEW",
                    table_name
                );
                self.py_execute_no_result(&sql::drop_view(&table_name))
                    .await?;
            } else {
                drop_result?;
            }
        }

        // After a Spark server restart, DROP TABLE IF EXISTS is a catalog no-op
        // (the table isn't in the new session's catalog) but the managed table's
        // warehouse directory persists on the host-visible bind mount (owned by
        // the Spark container's uid 185, not the host user).  Spark then refuses
        // CREATE TABLE with [LOCATION_ALREADY_EXISTS].
        //
        // Best-effort host-side cleanup: works when test fixtures were written by
        // the host process (e.g. seeds); silently skips when Spark owns the files.
        // For Spark-owned warehouse dirs, spark-up.sh cleans on each server restart
        // (the authoritative resolution — see scripts/spark-up.sh "Clean warehouse").
        if let Some(path) = self.materialized_path(schema, name) {
            if path.exists() {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => tracing::debug!(
                        "Removed stale warehouse directory {:?} before CREATE TABLE",
                        path
                    ),
                    Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                        tracing::debug!(
                            "Could not remove {:?} (owned by Spark uid): {:?}. \
                             Relying on spark-up.sh warehouse clean on next restart.",
                            path,
                            e
                        );
                    }
                    Err(e) => {
                        return Err(BackendError::Other(anyhow::anyhow!(
                            "Failed to remove stale warehouse directory {:?}: {}",
                            path,
                            e
                        )));
                    }
                }
            }
        }

        self.py_execute_no_result(&sql::create_table_as(
            &table_name,
            sql,
            if self.use_delta { Some("DELTA") } else { None },
        ))
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
        match self
            .py_execute_no_result(&sql::drop_table(&table_name))
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("WRONG_COMMAND_FOR_OBJECT_TYPE") || msg.contains("is a VIEW") {
                    // The name is occupied by a VIEW from a previous run; drop it instead.
                    self.py_execute_no_result(&sql::drop_view(&table_name))
                        .await
                } else {
                    Err(e)
                }
            }
        }
    }

    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        let view_name = self.qualified_name(schema, name);
        match self.py_execute_no_result(&sql::drop_view(&view_name)).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // Spark 4.x rejects DROP VIEW on a TABLE with WRONG_COMMAND_FOR_OBJECT_TYPE.
                // Silently succeed: the name is occupied by a table (which will be cleaned
                // up by the subsequent drop_table_if_exists call in execute_model).
                let msg = e.to_string();
                if msg.contains("WRONG_COMMAND_FOR_OBJECT_TYPE")
                    || msg.contains("DROP VIEW requires a VIEW")
                {
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
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

    fn materialized_path(&self, schema: &str, name: &str) -> Option<std::path::PathBuf> {
        // Spark stores managed tables under {warehouse}/{schema}.db/{table_name} —
        // the ".db" suffix is the Hive metastore convention used by Spark SQL when
        // spark.sql.warehouse.dir is set (empirically verified on Spark 4.1.x, W6·P3).
        self.warehouse
            .as_ref()
            .map(|wh| std::path::PathBuf::from(format!("{}/{}.db/{}", wh, schema, name)))
    }

    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, name);
        self.py_execute_no_result(&sql::delete_partitions_range(&table_name, partition))
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

    /// Overrides the trait default so the emitted `DELETE`/`INSERT` text
    /// targets the full catalog-qualified table name
    /// (`catalog.schema.table`) — the generic default in `smelt-backend`
    /// only sees `schema`/`name` and cannot know Spark's catalog. The text
    /// itself still comes from `emit_delete_insert`
    /// (`docs/specs/maintenance_plan.md` §"Statement emission (single
    /// owner)"); this crate builds no `DELETE`/`INSERT` string of its own.
    /// No native multi-statement transaction here (same precedent as the
    /// pre-Phase-1 default) — statements execute sequentially via
    /// `execute_statement_group`'s default.
    async fn delete_and_insert_transactional(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionRange,
        sql: &str,
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, name);
        let region = Region {
            start: format!("'{}'", partition.start.replace('\'', "''")),
            end: format!("'{}'", partition.end.replace('\'', "''")),
        };
        let group = emit_delete_insert(
            &table_name,
            &partition.column,
            &region,
            sql,
            MaintenanceDialect::Spark,
        );
        self.execute_statement_group(&group).await
    }

    // `merge_into` is not overridden here — the `Backend` trait's default
    // implementation (build the `StatementGroup` via `smelt_logical::
    // maintenance::emit::emit_column_scoped_merge`, then
    // `execute_statement_group`) routes through `execute_sql` per statement,
    // matching this backend's shape exactly (single statement, no native
    // transaction). `docs/specs/maintenance_plan.md` §"Statement emission
    // (single owner)" — Spark statement parity is asserted at the emitter
    // level; the live Spark leg is gated CI, not this default routing.

    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, table);
        self.py_execute_no_result(&sql::insert_overwrite(&table_name, sql, partition))
            .await
    }

    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), BackendError> {
        use arrow::ipc::writer::StreamWriter;
        use std::io::Cursor;

        let full_table_name = self.qualified_name(schema, name);

        // Pre-validate nullability against the authoritative `arrow_schema`.
        for batch in &batches {
            for (col_idx, field) in arrow_schema.fields().iter().enumerate() {
                if !field.is_nullable() {
                    let array = batch.column(col_idx);
                    if array.null_count() > 0 {
                        let row = (0..array.len()).find(|&i| array.is_null(i)).unwrap_or(0);
                        return Err(BackendError::null_in_non_nullable_column(
                            schema,
                            name,
                            field.name().as_str(),
                            row,
                        ));
                    }
                }
            }
        }

        // Serialize batches to Arrow IPC stream bytes — no host filesystem path.
        let mut ipc_buf = Vec::<u8>::new();
        {
            let cursor = Cursor::new(&mut ipc_buf);
            let mut writer = StreamWriter::try_new(cursor, &arrow_schema).map_err(|e| {
                BackendError::execution_failed(full_table_name.clone(), e.to_string())
            })?;
            for batch in &batches {
                writer.write(batch).map_err(|e| {
                    BackendError::execution_failed(full_table_name.clone(), e.to_string())
                })?;
            }
            writer.finish().map_err(|e| {
                BackendError::execution_failed(full_table_name.clone(), e.to_string())
            })?;
        }

        // Send IPC bytes across the PyO3 boundary; Python reconstructs the table
        // via pyarrow.ipc and loads it through createDataFrame (no host path).
        let adapter = Python::attach(|py| self.adapter.clone_ref(py));
        let full_table_name_clone = full_table_name.clone();

        tokio::task::spawn_blocking(move || {
            Python::attach(|py| {
                let ipc_bytes = pyo3::types::PyBytes::new(py, &ipc_buf);
                adapter
                    .call_method1(py, "load_arrow_table", (ipc_bytes, &full_table_name_clone))
                    .map_err(|e| {
                        BackendError::execution_failed(
                            full_table_name_clone.clone(),
                            format!("Spark load_arrow_table failed: {}", e),
                        )
                    })?;
                Ok(())
            })
        })
        .await
        .map_err(|e| BackendError::Other(anyhow::anyhow!("spawn_blocking join error: {}", e)))?
    }
}
