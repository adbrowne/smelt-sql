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
    /// `docs/specs/incremental_models.md` §Constraints "Never fold a delta
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

    /// Classify this error as transient (worth retrying, unchanged input) or
    /// deterministic (retrying is pointless — the same input will fail the
    /// same way).
    ///
    /// `docs/plans/20260719-prod-w2-operability.md` Phase 6: bounded retry
    /// wraps a model's statement-group execution and consults this
    /// classification to decide whether a failure is worth another attempt.
    /// This is an explicit, exhaustive match — **not** a string sniff over
    /// the error message — and deliberately has no wildcard arm: adding a
    /// new [`BackendError`] variant without adding a corresponding arm here
    /// is a compile error, forcing every new variant through an explicit
    /// transient/deterministic decision (the same fail-loud discipline
    /// `map_metadata_error_to_diagnostic` enforces for `MetadataError` in
    /// `smelt-db`, see root `CLAUDE.md` §"Fail-loud discipline").
    ///
    /// - `ConnectionFailed` — environmental (network blip, connection pool
    ///   exhaustion, backend not yet accepting connections): transient.
    /// - `ExecutionFailed` — a SQL statement the backend rejected (syntax,
    ///   type mismatch, constraint violation, division error, …): the same
    ///   SQL fails identically on retry, so deterministic.
    /// - `NotFound` / `SchemaNotFound` — a referenced object genuinely does
    ///   not exist; retrying without an intervening DDL change cannot help,
    ///   so deterministic.
    /// - `UnsupportedFeature` — a dialect gap; retrying cannot change what
    ///   the backend supports, so deterministic.
    /// - `NullInNonNullableColumn` — a data-quality violation in the source
    ///   data itself; retrying re-reads the same violating rows, so
    ///   deterministic.
    /// - `ConfigurationError` — a misconfiguration (bad connection string,
    ///   missing credential, …) that retrying does not fix, so
    ///   deterministic.
    /// - `AlreadyReflected` — a deliberate never-fold-twice refusal, not a
    ///   failure to retry past; deterministic.
    /// - `Other` — an unclassified `anyhow` error from a backend-specific
    ///   code path (e.g. seed loading, ledger bookkeeping). Without a typed
    ///   variant to inspect, treating it as transient would risk masking a
    ///   deterministic failure behind retries; deterministic is the
    ///   fail-loud choice — a backend that wants a transient `Other` retried
    ///   should be given a typed variant instead of relying on this
    ///   fallback.
    pub fn is_transient(&self) -> bool {
        match self {
            BackendError::ConnectionFailed { .. } => true,
            BackendError::ExecutionFailed { .. } => false,
            BackendError::NotFound { .. } => false,
            BackendError::SchemaNotFound { .. } => false,
            BackendError::UnsupportedFeature { .. } => false,
            BackendError::NullInNonNullableColumn { .. } => false,
            BackendError::ConfigurationError { .. } => false,
            BackendError::AlreadyReflected { .. } => false,
            BackendError::Other(_) => false,
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
