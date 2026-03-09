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

    #[error("Model '{model}' uses named parameters which are not yet supported\n\n  --> {file}:{line}:{col}\n   |\n{snippet}\n   |\n   = note: Named parameters will be supported in a future release\n   = help: For now, use: FROM smelt.ref('model_name') without parameters")]
    NamedParametersNotSupported {
        model: String,
        file: PathBuf,
        line: u32,
        col: u32,
        snippet: String,
    },

    #[error("Python interpreter not found.\nInstall Python 3 or set the SMELT_PYTHON environment variable.")]
    PythonNotFound,

    #[error("Python model error in {file}:\n{message}")]
    PythonExecutionError { file: PathBuf, message: String },
}
