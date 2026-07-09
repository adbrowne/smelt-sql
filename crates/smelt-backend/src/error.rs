//! Backend error types.

use thiserror::Error;

/// Errors that can occur during backend operations.
#[derive(Error, Debug)]
pub enum BackendError {
    /// Failed to connect to the backend.
    #[error("Connection failed: {message}")]
    ConnectionFailed { message: String },

    /// Failed to execute a SQL query.
    #[error("Execution failed for '{model}': {message}")]
    ExecutionFailed { model: String, message: String },

    /// Table or view not found.
    #[error("Table or view not found: {schema}.{name}")]
    NotFound { schema: String, name: String },

    /// Schema does not exist.
    #[error("Schema not found: {schema}")]
    SchemaNotFound { schema: String },

    /// SQL dialect feature not supported.
    #[error("Feature not supported by {dialect}: {feature}")]
    UnsupportedFeature { dialect: String, feature: String },

    /// NULL value found in a column declared as NOT NULL (nullable: false).
    #[error("NULL value in non-nullable column '{column}' at row {row} (table {schema}.{table})")]
    NullInNonNullableColumn {
        schema: String,
        table: String,
        column: String,
        row: usize,
    },

    /// Configuration error.
    #[error("Configuration error: {message}")]
    ConfigurationError { message: String },

    /// A fold was refused because the delta is already reflected in the
    /// warehouse-resident reconciliation ledger (never-fold-twice,
    /// `docs/specs/maintenance_plan.md` §Constraints "Never fold a delta
    /// already reflected in the state"). Distinct from `ExecutionFailed` so
    /// callers can surface a `KeyedReprocessedWindow`-shaped refusal instead
    /// of a generic execution error.
    #[error("delta already reflected in the reconciliation ledger: {message}")]
    AlreadyReflected { message: String },

    /// Generic backend error.
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl BackendError {
    /// Create a connection failed error.
    pub fn connection_failed(message: impl Into<String>) -> Self {
        Self::ConnectionFailed {
            message: message.into(),
        }
    }

    /// Create an execution failed error.
    pub fn execution_failed(model: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ExecutionFailed {
            model: model.into(),
            message: message.into(),
        }
    }

    /// Create a not found error.
    pub fn not_found(schema: impl Into<String>, name: impl Into<String>) -> Self {
        Self::NotFound {
            schema: schema.into(),
            name: name.into(),
        }
    }

    /// Create an unsupported feature error.
    pub fn unsupported(dialect: impl Into<String>, feature: impl Into<String>) -> Self {
        Self::UnsupportedFeature {
            dialect: dialect.into(),
            feature: feature.into(),
        }
    }

    /// Create an already-reflected (never-fold-twice refusal) error.
    pub fn already_reflected(message: impl Into<String>) -> Self {
        Self::AlreadyReflected {
            message: message.into(),
        }
    }

    /// Create a NULL-in-non-nullable-column error.
    pub fn null_in_non_nullable_column(
        schema: impl Into<String>,
        table: impl Into<String>,
        column: impl Into<String>,
        row: usize,
    ) -> Self {
        Self::NullInNonNullableColumn {
            schema: schema.into(),
            table: table.into(),
            column: column.into(),
            row,
        }
    }
}
