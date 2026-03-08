use rowan::TextRange;
use std::path::PathBuf;
use thiserror::Error;

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

    #[error("Dependency resolution failed:\n  {message}")]
    DependencyError { message: String },

    #[error("Parse error in {file}:{line}:{col}\n  {message}")]
    ParseError {
        file: String,
        line: u32,
        col: u32,
        message: String,
    },

    #[error("Circular dependency detected involving models: {models}")]
    CircularDependency { models: String },

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

/// Helper to convert TextRange to line/column for error messages
pub fn text_range_to_line_col(text: &str, range: TextRange) -> (u32, u32) {
    let offset: usize = range.start().into();
    let mut line = 0u32;
    let mut col = 0u32;

    for (idx, ch) in text.chars().enumerate() {
        if idx >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }

    (line, col)
}

/// Helper to extract a snippet of code for error messages
pub fn extract_snippet(text: &str, range: TextRange, context_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let (target_line, _) = text_range_to_line_col(text, range);

    let start_line = target_line.saturating_sub(context_lines as u32) as usize;
    let end_line = ((target_line + context_lines as u32 + 1) as usize).min(lines.len());

    let snippet_lines: Vec<String> = lines[start_line..end_line]
        .iter()
        .enumerate()
        .map(|(idx, line)| {
            let line_num = start_line + idx + 1;
            format!("{:4} | {}", line_num, line)
        })
        .collect();

    snippet_lines.join("\n")
}
