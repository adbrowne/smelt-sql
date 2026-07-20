use std::path::PathBuf;
use thiserror::Error;

// Re-export text utilities from smelt-core
pub use smelt_core::{extract_snippet, text_range_to_line_col};

#[derive(Debug, Error)]
pub enum CliError {
    #[error("Could not find smelt project root.\nExpected to find 'smelt.yml' or 'models/' directory.\nHint: Run 'smelt init' to create a new project.")]
    ProjectRootNotFound,

    #[error("Failed to load configuration file: {path}\n{source}")]
    ConfigLoadError {
        path: PathBuf,
        source: anyhow::Error,
    },

    #[error("Model '{model}' failed to compile:\n  {source}")]
    CompilationError {
        model: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Model '{model}' failed to execute:\n  {source}\n\nSQL:\n{sql}")]
    ExecutionError {
        model: String,
        sql: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Parse error in {file}:{line}:{col}\n  {message}")]
    ParseError {
        file: String,
        line: u32,
        col: u32,
        message: String,
    },

    #[error("Source tables not found in database:\n  {}\n\nHint: Create source tables manually or use 'smelt seed' command", missing.join("\n  "))]
    SourceTablesNotFound { missing: Vec<String> },

    #[error("Python interpreter not found.\nInstall Python 3 or set the SMELT_PYTHON environment variable.")]
    PythonNotFound,

    #[error("Python model error in {file}:\n{message}")]
    PythonExecutionError { file: PathBuf, message: String },

    #[error("Seed error for '{name}': {source}")]
    SeedError {
        name: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("No seed files found in seed paths: {}", paths.join(", "))]
    NoSeedsFound { paths: Vec<String> },

    /// A command ran to completion but detected a failure in the data or
    /// models being processed (a failed build/test/check, a schema diff).
    /// Distinct from a usage/config error: see [`exit_code_for`].
    #[error("{0}")]
    DetectedFailure(String),
}

/// Classify a top-level command error into the exit code contract in
/// `docs/specs/cli.md` §"Exit codes": `2` for usage/config errors (the
/// command could not run at all because its own inputs were invalid), `1`
/// for everything else (the command ran and found a problem).
///
/// `anyhow::Error::downcast_ref` searches the entire source chain (not just
/// the outermost wrapper), so this classifies correctly even though every
/// call site attaches `.with_context(...)` on top of the originating error.
pub fn exit_code_for(err: &anyhow::Error) -> u8 {
    if err
        .downcast_ref::<smelt_core::project::ProjectError>()
        .is_some()
        || err
            .downcast_ref::<smelt_core::config::ConfigError>()
            .is_some()
    {
        2
    } else {
        1
    }
}
