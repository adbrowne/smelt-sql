//! Backend trait and types for smelt execution engines.
//!
//! This crate defines the abstract interface that all smelt backends must implement,
//! enabling multi-backend support (DuckDB, Spark, etc.).

use tracing::warn;

mod error;
mod types;

pub use error::BackendError;
pub use smelt_core::config::{
    Granularity, IncrementalConfig, IncrementalSafetyOverrides, IncrementalStrategy,
};
pub use smelt_dialect::{BackendCapabilities, SqlDialect};
pub use types::{
    ExecutionResult, Materialization, MaterializationStrategy, PartitionRange, PartitionSpec,
};

use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;

/// Abstract interface for smelt execution backends.
///
/// Backends are responsible for:
/// - Executing SQL queries
/// - Creating tables and views
/// - Validating source tables exist
/// - Reporting their SQL dialect and capabilities
#[async_trait]
pub trait Backend: Send + Sync {
    /// Execute a SQL query and return results.
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError>;

    /// Create a table from a SQL query.
    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError>;

    /// Create a view from a SQL query.
    async fn create_view_as(&self, schema: &str, name: &str, sql: &str)
        -> Result<(), BackendError>;

    /// Drop a table if it exists.
    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError>;

    /// Drop a view if it exists.
    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError>;

    /// Get the row count of a table or view.
    async fn get_row_count(&self, schema: &str, name: &str) -> Result<usize, BackendError>;

    /// Get a preview of a table or view (first N rows).
    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, BackendError>;

    /// Check if a table exists.
    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError>;

    /// Ensure a schema exists, creating it if necessary.
    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError>;

    /// Get the SQL dialect this backend uses.
    fn dialect(&self) -> SqlDialect;

    /// Get the capabilities of this backend.
    fn capabilities(&self) -> BackendCapabilities;

    /// Load Arrow record batches into a new table at `schema.name`.
    ///
    /// Drops any existing table or view with the same name first (like seed loading does).
    /// Returns an error if a non-nullable column (Arrow `Field::nullable == false`) contains
    /// NULL values.
    ///
    /// Supported Arrow types (matching the seed type set from `seeds.md §"Type inference"`):
    /// - `DataType::Boolean` → `BOOLEAN`
    /// - `DataType::Int32` → `INTEGER`
    /// - `DataType::Int64` → `BIGINT`
    /// - `DataType::Decimal128(p, s)` → `DECIMAL(p, s)` with p ≤ 18 and s ≤ 4
    /// - `DataType::Float64` → `DOUBLE`
    /// - `DataType::Date32` → `DATE`
    /// - `DataType::Timestamp(TimeUnit::Microsecond, None)` → `TIMESTAMP` (no TZ)
    /// - `DataType::Utf8` → `VARCHAR`
    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), BackendError>;

    /// Get the filesystem path where a model's output is materialized.
    /// Returns None for database backends (DuckDB), Some(path) for file-based backends (Spark).
    fn materialized_path(&self, schema: &str, name: &str) -> Option<std::path::PathBuf> {
        let _ = (schema, name);
        None
    }

    /// Execute a model (drop + create as table or view).
    ///
    /// This is a convenience method that combines drop + create operations.
    async fn execute_model(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
        materialization: Materialization,
        show_preview: bool,
    ) -> Result<ExecutionResult, BackendError> {
        let start = std::time::Instant::now();

        match materialization {
            Materialization::Table => {
                // Drop both view and table in case the materialization type changed
                self.drop_view_if_exists(schema, name).await?;
                self.drop_table_if_exists(schema, name).await?;
                self.create_table_as(schema, name, sql).await?;
            }
            Materialization::View => {
                // Drop both table and view in case the materialization type changed
                self.drop_table_if_exists(schema, name).await?;
                self.drop_view_if_exists(schema, name).await?;
                self.create_view_as(schema, name, sql).await?;
            }
            Materialization::MaterializedView => {
                if self.capabilities().supports_materialized_views {
                    self.drop_table_if_exists(schema, name).await?;
                    self.drop_view_if_exists(schema, name).await?;
                    self.drop_materialized_view_if_exists(schema, name).await?;
                    self.create_materialized_view_as(schema, name, sql).await?;
                } else {
                    warn!(
                        "backend doesn't support materialized views, using table for '{}'",
                        name
                    );
                    self.drop_view_if_exists(schema, name).await?;
                    self.drop_table_if_exists(schema, name).await?;
                    self.create_table_as(schema, name, sql).await?;
                }
            }
        }

        let duration = start.elapsed();
        let row_count = self.get_row_count(schema, name).await?;

        let preview = if show_preview {
            Some(self.get_preview(schema, name, 10).await?)
        } else {
            None
        };

        Ok(ExecutionResult {
            model_name: name.to_string(),
            duration,
            row_count,
            preview,
        })
    }

    /// Execute a model with incremental materialization support.
    ///
    /// Dispatches to the appropriate strategy (DELETE+INSERT, MERGE, APPEND, INSERT OVERWRITE).
    async fn execute_model_incremental(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
        materialization: Materialization,
        strategy: MaterializationStrategy,
        show_preview: bool,
    ) -> Result<ExecutionResult, BackendError> {
        let start = std::time::Instant::now();

        match (materialization, strategy) {
            (Materialization::View | Materialization::MaterializedView, _) => {
                // Views and materialized views can't be incremental — full refresh
                self.execute_model(schema, name, sql, materialization, show_preview)
                    .await?;
                // Return early to avoid double row count/preview
                return Ok(ExecutionResult {
                    model_name: name.to_string(),
                    duration: start.elapsed(),
                    row_count: self.get_row_count(schema, name).await?,
                    preview: if show_preview {
                        Some(self.get_preview(schema, name, 10).await?)
                    } else {
                        None
                    },
                });
            }
            (Materialization::Table, MaterializationStrategy::FullRefresh) => {
                self.drop_table_if_exists(schema, name).await?;
                self.create_table_as(schema, name, sql).await?;
            }
            (
                Materialization::Table,
                MaterializationStrategy::Incremental {
                    partition,
                    strategy: inc_strategy,
                    unique_key,
                },
            ) => {
                let table_exists = self.table_exists(schema, name).await?;

                if !table_exists {
                    self.create_table_as(schema, name, sql).await?;
                } else {
                    let _ = unique_key; // unused since the cumulative path owns merge_into
                    match inc_strategy {
                        IncrementalStrategy::DeleteInsert => {
                            self.delete_partitions(schema, name, &partition).await?;
                            self.insert_into_from_query(schema, name, sql).await?;
                        }
                        IncrementalStrategy::Append => {
                            self.insert_into_from_query(schema, name, sql).await?;
                        }
                        IncrementalStrategy::InsertOverwrite => {
                            self.insert_overwrite(schema, name, sql, &partition).await?;
                        }
                    }
                }
            }
        }

        let duration = start.elapsed();
        let row_count = self.get_row_count(schema, name).await?;

        let preview = if show_preview {
            Some(self.get_preview(schema, name, 10).await?)
        } else {
            None
        };

        Ok(ExecutionResult {
            model_name: name.to_string(),
            duration,
            row_count,
            preview,
        })
    }

    /// Resolve the best incremental strategy for the given config.
    ///
    /// Default implementation: always returns `DeleteInsert`. MERGE is no
    /// longer an incremental strategy — it is the physical primitive of the
    /// `cumulative_aggregate` materialization (see
    /// `docs/specs/cumulative_aggregate.md`). The `unique_key` field on
    /// `IncrementalConfig` is reserved for backends that may want to use it
    /// for diagnostics or audit; it does not change strategy selection here.
    fn resolve_strategy(&self, _config: &IncrementalConfig) -> IncrementalStrategy {
        IncrementalStrategy::DeleteInsert
    }

    /// Delete rows in a half-open partition range `[start, end)`.
    ///
    /// Emits `DELETE FROM table WHERE column >= start AND column < end`.
    /// This form is both more efficient than an IN-list for large windows and
    /// is correct for any window size without enumerating individual partition
    /// values.
    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError>;

    /// Insert data from a SELECT query into an existing table.
    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError>;

    /// MERGE (upsert) rows using unique_key columns for matching.
    ///
    /// Matched rows are updated, unmatched rows are inserted.
    async fn merge_into(
        &self,
        schema: &str,
        table: &str,
        source_sql: &str,
        unique_key: &[String],
    ) -> Result<(), BackendError>;

    /// Replace partitions by inserting new data and removing old partition rows.
    ///
    /// Backends with native INSERT OVERWRITE use that; others emulate via
    /// DELETE (partition values derived from source) + INSERT.
    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError>;

    /// Create a materialized view from a SQL query.
    ///
    /// Default implementation falls back to `create_table_as` with a warning.
    async fn create_materialized_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        warn!("backend does not support materialized views, falling back to table");
        self.create_table_as(schema, name, sql).await
    }

    /// Drop a materialized view if it exists.
    ///
    /// Default implementation falls back to `drop_table_if_exists`.
    async fn drop_materialized_view_if_exists(
        &self,
        schema: &str,
        name: &str,
    ) -> Result<(), BackendError> {
        self.drop_table_if_exists(schema, name).await
    }
}
