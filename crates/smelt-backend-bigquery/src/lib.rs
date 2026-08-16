//! BigQuery backend implementation for smelt via a google-cloud-bigquery/PyO3 bridge.
//!
//! Mirrors `smelt-backend-spark`: PyO3 calls a thin Python adapter
//! (`python/smelt/bigquery_adapter.py`) which holds the client and returns Arrow
//! record batches. All SQL generation and orchestration stays in Rust.
//!
//! Authentication is from an explicitly-supplied OAuth access token and never
//! from Google application-default credentials (`docs/specs/multi_backend.md`
//! §Surface — "BigQuery authenticates from an explicit token"). The refusal is
//! enforced in the adapter's constructor, so no code path here can reach ambient
//! credentials.

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

/// BigQuery backend for smelt, powered by google-cloud-bigquery via PyO3.
///
/// Holds a Python `BigQueryAdapter` object. All Python calls go through
/// `spawn_blocking` to avoid blocking the async runtime.
pub struct BigQueryBackend {
    adapter: Py<PyAny>,
    project: String,
    dataset: String,
}

// Safety: Py<PyAny> is Send, and we only access it inside `Python::attach`
// which acquires the GIL. All access is serialized through spawn_blocking.
unsafe impl Send for BigQueryBackend {}
unsafe impl Sync for BigQueryBackend {}

impl BigQueryBackend {
    /// Create a new BigQuery backend.
    ///
    /// # Arguments
    /// * `project` - GCP project id jobs are billed to
    /// * `dataset` - Dataset holding this target's tables (the schema analogue)
    /// * `location` - Dataset location (e.g. "US"); must match at query time
    /// * `access_token` - Short-lived OAuth token. Required: there is no
    ///   application-default-credentials fallback, by design.
    pub async fn new(
        project: &str,
        dataset: &str,
        location: Option<&str>,
        access_token: &str,
    ) -> Result<Self, BackendError> {
        let project = project.to_string();
        let dataset = dataset.to_string();
        let location = location.map(|s| s.to_string());
        let access_token = access_token.to_string();

        let adapter = tokio::task::spawn_blocking({
            let project = project.clone();
            let dataset = dataset.clone();
            move || {
                Python::attach(|py| {
                    let module = py.import("smelt.bigquery_adapter").map_err(|e| {
                        BackendError::connection_failed(format!(
                            "Failed to import smelt.bigquery_adapter: {}. \
                             Create the client venv with `bash scripts/bigquery-venv.sh`, \
                             then `source scripts/bigquery-env.sh` to put it on PYTHONPATH.",
                            e
                        ))
                    })?;

                    let cls = module.getattr("BigQueryAdapter").map_err(|e| {
                        BackendError::connection_failed(format!(
                            "BigQueryAdapter class not found: {}",
                            e
                        ))
                    })?;

                    let adapter = cls
                        .call1((&project, &dataset, location, &access_token))
                        .map_err(|e| {
                            BackendError::connection_failed(format!(
                                "Failed to create BigQuery client: {}",
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
            project,
            dataset: dataset.clone(),
        };

        // Create the dataset before anything selects it (spec: multi_backend.md
        // §Semantics "Session initialization" — requires_schema_init = true).
        // BigQuery refuses any write into a dataset that does not exist, so a
        // fresh warehouse would otherwise fail on the first model.
        if !dataset.is_empty() {
            backend.ensure_schema(&dataset).await?;
        }

        tracing::info!(
            "BigQuery session established (project={}, dataset={})",
            backend.project,
            backend.dataset
        );

        Ok(backend)
    }

    /// Build a fully qualified, backtick-quoted name: `` `project.dataset.table` ``.
    fn qualified_name(&self, schema: &str, name: &str) -> String {
        sql::qualified_name(&self.project, schema, name)
    }

    /// Execute SQL via the Python adapter, returning Arrow RecordBatches.
    async fn py_execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        let sql = sql.to_string();
        let adapter = Python::attach(|py| self.adapter.clone_ref(py));

        tokio::task::spawn_blocking(move || {
            Python::attach(|py| {
                let result = adapter
                    .call_method1(py, "execute_sql", (&sql,))
                    .map_err(|e| {
                        BackendError::execution_failed("bigquery sql", format!("{}", e))
                    })?;

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
                    .map_err(|e| {
                        BackendError::execution_failed("bigquery sql", format!("{}", e))
                    })?;
                Ok(())
            })
        })
        .await
        .map_err(|e| BackendError::Other(anyhow::anyhow!("spawn_blocking join error: {}", e)))?
    }
}

#[async_trait]
impl Backend for BigQueryBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        tracing::debug!("BigQuery execute_sql: {}", sql::truncate_sql(sql));
        self.py_execute_sql(sql).await
    }

    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let table_name = self.qualified_name(schema, name);
        tracing::debug!("BigQuery CREATE OR REPLACE TABLE {} AS ...", table_name);
        // Native CREATE OR REPLACE TABLE — no DROP-then-CREATE emulation needed.
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
        tracing::debug!("BigQuery CREATE OR REPLACE VIEW {} AS ...", view_name);
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
        // The adapter's get_row_count quotes the name itself, so pass the
        // unquoted three-part form.
        let table_name = format!("{}.{}.{}", self.project, schema, name);
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
        self.py_execute_sql(&sql::select_preview(&table_name, limit))
            .await
    }

    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        let table_name = format!("{}.{}.{}", self.project, schema, name);
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

    /// Create the dataset if absent.
    ///
    /// Goes through the client API rather than `CREATE SCHEMA` so the dataset's
    /// location is set at creation — a dataset created in the wrong location
    /// cannot be queried alongside the rest of the target's tables.
    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        let schema = schema.to_string();
        let adapter = Python::attach(|py| self.adapter.clone_ref(py));

        tokio::task::spawn_blocking(move || {
            Python::attach(|py| {
                adapter
                    .call_method1(py, "ensure_schema", (&schema,))
                    .map_err(|e| {
                        BackendError::connection_failed(format!(
                            "Failed to ensure dataset '{}': {}",
                            schema, e
                        ))
                    })?;
                Ok(())
            })
        })
        .await
        .map_err(|e| BackendError::Other(anyhow::anyhow!("spawn_blocking join error: {}", e)))?
    }

    fn dialect(&self) -> SqlDialect {
        SqlDialect::BigQuery
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::bigquery()
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

    /// BigQuery has no `INSERT OVERWRITE` (`supports_insert_overwrite = false`,
    /// verified live), so partition replacement lowers to the scoped
    /// `DELETE` + `INSERT` pair rather than surfacing an error — the
    /// lower-don't-reject rule in `multi_backend.md` §"Parity contract".
    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.delete_and_insert_transactional(schema, table, partition, sql)
            .await
    }

    /// Overrides the trait default so the emitted `DELETE`/`INSERT` text targets
    /// the backtick-quoted three-part name — the generic default only sees
    /// `schema`/`name` and cannot know the project. The text itself still comes
    /// from `emit_delete_insert` (`docs/specs/incremental_models.md` §"Statement
    /// emission (single owner)"); this crate authors no DELETE/INSERT of its own.
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
            MaintenanceDialect::BigQuery,
        );
        self.execute_statement_group(&group).await
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

        // The adapter's load path quotes nothing (it targets the client API),
        // so pass the unquoted three-part name.
        let full_table_name = format!("{}.{}.{}", self.project, schema, name);

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

        // Serialize batches to Arrow IPC stream bytes — no host filesystem path
        // (`multi_backend.md` §"Loading data into a backend").
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
                            format!("BigQuery load_arrow_table failed: {}", e),
                        )
                    })?;
                Ok(())
            })
        })
        .await
        .map_err(|e| BackendError::Other(anyhow::anyhow!("spawn_blocking join error: {}", e)))?
    }
}
