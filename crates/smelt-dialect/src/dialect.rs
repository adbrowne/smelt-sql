//! SQL dialect definitions and backend capabilities.

/// SQL dialect used by a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDialect {
    /// DuckDB SQL dialect
    DuckDB,
    /// Apache Spark SQL dialect
    SparkSQL,
    /// PostgreSQL dialect
    PostgreSQL,
}

impl SqlDialect {
    /// Get a human-readable name for this dialect.
    pub fn name(&self) -> &'static str {
        match self {
            SqlDialect::DuckDB => "DuckDB",
            SqlDialect::SparkSQL => "Spark SQL",
            SqlDialect::PostgreSQL => "PostgreSQL",
        }
    }
}

/// Capabilities of a backend.
///
/// Used to determine what SQL features can be used directly vs. need rewriting.
#[derive(Debug, Clone)]
pub struct BackendCapabilities {
    /// Supports QUALIFY clause for window function filtering
    pub supports_qualify: bool,

    /// Supports CREATE OR REPLACE TABLE syntax
    pub supports_create_or_replace_table: bool,

    /// Supports CREATE OR REPLACE VIEW syntax
    pub supports_create_or_replace_view: bool,

    /// Supports MERGE statement (upsert)
    pub supports_merge: bool,

    /// Supports PIVOT/UNPIVOT natively
    pub supports_pivot: bool,

    /// Supports DATE 'YYYY-MM-DD' literal syntax
    pub supports_date_literal: bool,

    /// Supports || for string concatenation
    pub supports_concat_operator: bool,

    /// Supports arrays with [a, b, c] syntax
    pub supports_array_literal: bool,

    /// Supports transactional DDL (can rollback CREATE TABLE)
    pub supports_transactional_ddl: bool,
}

impl BackendCapabilities {
    /// Capabilities for DuckDB
    pub fn duckdb() -> Self {
        Self {
            supports_qualify: true,
            supports_create_or_replace_table: true,
            supports_create_or_replace_view: true,
            supports_merge: true,
            supports_pivot: true,
            supports_date_literal: true,
            supports_concat_operator: true,
            supports_array_literal: true,
            supports_transactional_ddl: true,
        }
    }

    /// Capabilities for Spark SQL
    pub fn spark() -> Self {
        Self {
            supports_qualify: false,                 // Requires subquery rewrite
            supports_create_or_replace_table: false, // DROP + CREATE
            supports_create_or_replace_view: true,
            supports_merge: true, // Delta Lake only
            supports_pivot: true,
            supports_date_literal: false, // Uses DATE('YYYY-MM-DD') function
            supports_concat_operator: true,
            supports_array_literal: false, // Uses ARRAY(a, b, c)
            supports_transactional_ddl: false,
        }
    }

    /// Capabilities for PostgreSQL
    pub fn postgresql() -> Self {
        Self {
            supports_qualify: false,                 // Requires subquery rewrite
            supports_create_or_replace_table: false, // DROP + CREATE
            supports_create_or_replace_view: true,
            supports_merge: true,  // PostgreSQL 15+
            supports_pivot: false, // Requires crosstab extension
            supports_date_literal: true,
            supports_concat_operator: true,
            supports_array_literal: false, // Uses ARRAY[a, b, c]
            supports_transactional_ddl: true,
        }
    }
}
