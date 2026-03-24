//! Common types used across backends.

use arrow::array::RecordBatch;
use smelt_core::config::IncrementalStrategy;
use std::time::Duration;

/// Result of executing a model.
#[derive(Debug)]
pub struct ExecutionResult {
    /// Name of the model that was executed.
    pub model_name: String,

    /// How long execution took.
    pub duration: Duration,

    /// Number of rows in the resulting table/view.
    pub row_count: usize,

    /// Optional preview of the first few rows.
    pub preview: Option<Vec<RecordBatch>>,
}

/// How a model should be materialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Materialization {
    /// Materialize as a table (persisted).
    Table,

    /// Materialize as a view (computed on query).
    #[default]
    View,
}

impl std::fmt::Display for Materialization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Materialization::Table => write!(f, "table"),
            Materialization::View => write!(f, "view"),
        }
    }
}

impl std::str::FromStr for Materialization {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "table" => Ok(Materialization::Table),
            "view" => Ok(Materialization::View),
            _ => Err(format!("Unknown materialization: {}", s)),
        }
    }
}

/// Partition specification for incremental updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionSpec {
    /// Column name used for partitioning (e.g., "date", "event_date")
    pub column: String,

    /// Partition values to update (e.g., vec!["2024-01-01", "2024-01-02"])
    pub values: Vec<String>,
}

/// Materialization strategy for tables.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MaterializationStrategy {
    /// Full refresh: DROP + CREATE (existing behavior)
    #[default]
    FullRefresh,

    /// Incremental: strategy-dependent update
    Incremental {
        partition: PartitionSpec,
        strategy: IncrementalStrategy,
        /// Columns that uniquely identify a row (used by Merge strategy)
        unique_key: Vec<String>,
    },
}
