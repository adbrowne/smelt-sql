//! Execute a Python model file via the embedded PyO3 interpreter — the path
//! used when the `python` feature is enabled (no subprocess spawn).

use anyhow::{anyhow, Result};
use std::path::Path;

use super::PythonModelOutput;

/// Execute a Python model file via embedded PyO3 interpreter.
pub(super) fn run_python_model(
    _python: &str,
    file_path: &Path,
    project_context_json: &str,
    python_sdk_path: &Path,
) -> Result<Vec<PythonModelOutput>> {
    use pyo3::prelude::*;
    use pyo3::types::PyTracebackMethods;

    Python::attach(|py| {
        smelt_core::python_models::ensure_sdk_on_path(py, python_sdk_path)
            .map_err(|e| anyhow!("Failed to set up Python SDK path: {}", e))?;

        let outputs =
            smelt_core::python_models::run_python_model_file(py, file_path, project_context_json)
                .map_err(|e| {
                // Get traceback for better error messages
                let tb = e
                    .traceback(py)
                    .map(|tb| tb.format().unwrap_or_default())
                    .unwrap_or_default();
                anyhow!(
                    "Python model error in {}:\n{}{}",
                    file_path.display(),
                    tb,
                    e
                )
            })?;

        Ok(outputs
            .into_iter()
            .map(|o| PythonModelOutput {
                name: o.name,
                sql: o.sql,
                queries: o.queries,
            })
            .collect())
    })
}
