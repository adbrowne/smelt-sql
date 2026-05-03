//! Error types for seed CSV parsing and type coercion.

use smelt_types::DataType;
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during seed CSV parsing and loading.
#[derive(Debug, Error)]
pub enum SeedError {
    /// CSV file could not be read (I/O error).
    #[error("failed to read seed file {path}: {message}")]
    Io { path: PathBuf, message: String },

    /// CSV parse error (invalid quoting, unexpected field count, etc.).
    #[error("CSV parse error in {path} at line {line}: {message}")]
    CsvParseError {
        path: PathBuf,
        line: usize,
        message: String,
    },

    /// File uses the wrong field delimiter (e.g., tab instead of comma).
    ///
    /// Strict-defaults rule: only comma-delimited files are accepted.
    #[error("CSV file {path} appears to use a non-comma delimiter (found {found:?} in header); only comma-delimited files are accepted")]
    WrongDelimiter { path: PathBuf, found: char },

    /// A value that could not be coerced to the declared column type.
    #[error(
        "type coercion failed in {path} at row {row}, column {column}: \
        cannot coerce {value:?} to {expected}"
    )]
    TypeCoercionFailed {
        path: PathBuf,
        row: usize,
        column: String,
        value: String,
        expected: DataType,
    },
}
