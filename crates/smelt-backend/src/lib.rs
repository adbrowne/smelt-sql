//! Backend trait and types for smelt execution engines.
//!
//! This crate defines the abstract interface that all smelt backends must implement,
//! enabling multi-backend support (DuckDB, Spark, etc.).

mod error;
mod types;

pub use error::BackendError;
pub use smelt_core::config::{
    Granularity, IncrementalConfig, IncrementalSafetyOverrides, IncrementalStrategy,
};
pub use smelt_dialect::{BackendCapabilities, SqlDialect};
pub use types::{ExecutionResult, Materialization, MaterializationStrategy, PartitionSpec};

use arrow::array::RecordBatch;
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
                self.drop_table_if_exists(schema, name).await?;
                self.create_table_as(schema, name, sql).await?;
            }
            Materialization::View => {
                self.drop_view_if_exists(schema, name).await?;
                self.create_view_as(schema, name, sql).await?;
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
            (Materialization::View, _) => {
                self.drop_view_if_exists(schema, name).await?;
                self.create_view_as(schema, name, sql).await?;
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
                    match inc_strategy {
                        IncrementalStrategy::DeleteInsert => {
                            self.delete_partitions(schema, name, &partition).await?;
                            self.insert_into_from_query(schema, name, sql).await?;
                        }
                        IncrementalStrategy::Merge => {
                            self.merge_into(schema, name, sql, &unique_key).await?;
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
    /// Default implementation: uses MERGE if unique_key is set and supported,
    /// otherwise falls back to DELETE+INSERT.
    fn resolve_strategy(&self, config: &IncrementalConfig) -> IncrementalStrategy {
        if !config.unique_key.is_empty() && self.capabilities().supports_merge {
            IncrementalStrategy::Merge
        } else {
            IncrementalStrategy::DeleteInsert
        }
    }

    /// Delete rows matching partition values.
    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionSpec,
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
        partition: &PartitionSpec,
    ) -> Result<(), BackendError>;
}
