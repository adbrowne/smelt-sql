//! Shared PyO3-based Python model execution.
//!
//! When the `python` feature is enabled, this module provides in-process
//! execution of Python model files via PyO3, eliminating subprocess overhead.
//! Used by both CLI and LSP for Python model discovery.

use pyo3::prelude::*;
use pyo3::types::PyList;
use std::path::Path;

mod registry_lock;
#[cfg(test)]
mod tests;

use registry_lock::lock_registry;

/// Output from a single Python model function executed via PyO3.
#[derive(Debug, Clone)]
pub struct PythonModelOutput {
    pub name: String,
    pub sql: String,
    pub queries: Vec<PythonModelQuery>,
}

// `PythonModelQuery` is defined in `crate::discovery` (non-feature-gated) so
// the CLI's discovery can attach it to `ModelFile` without dragging PyO3 in.
pub use crate::discovery::PythonModelQuery;

/// Execute a Python model file in-process via PyO3.
///
/// This mirrors `runner.py` exactly but runs in the embedded interpreter:
/// 1. Ensure the model file's parent directory is on `sys.path`
/// 2. Clear `smelt.core._registered_models`
/// 3. Create `ProjectContext` from the JSON
/// 4. Load and exec the model file using `importlib.util`
/// 5. Iterate `_registered_models`, call each function with the context
/// 6. Extract `(name, sql, queries)` from results
pub fn run_python_model_file(
    py: Python<'_>,
    file_path: &Path,
    project_context_json: &str,
) -> PyResult<Vec<PythonModelOutput>> {
    // Hold the registry lock for the whole clear/exec/collect section. Released
    // GIL while blocking — see `REGISTRY_HELD`.
    let _registry_guard = lock_registry(py);

    let sys = py.import("sys")?;
    let sys_path = sys.getattr("path")?;

    // Add model file's parent directory to sys.path so sibling imports work
    if let Some(parent) = file_path.parent() {
        let parent_str = parent.to_string_lossy().to_string();
        let contains: bool = sys_path
            .call_method1("__contains__", (&parent_str,))?
            .extract()?;
        if !contains {
            sys_path.call_method1("insert", (0, &parent_str))?;
        }
    }

    // Import smelt.core and clear registered models
    let smelt_core = py.import("smelt.core")?;
    let registered_models = smelt_core.getattr("_registered_models")?;
    registered_models.call_method0("clear")?;

    // Create ProjectContext from JSON
    let json_mod = py.import("json")?;
    let project_data = json_mod.call_method1("loads", (project_context_json,))?;
    let models_list = project_data.call_method1("get", ("models", PyList::empty(py)))?;
    let project_context_cls = smelt_core.getattr("ProjectContext")?;
    let project_context = project_context_cls.call1((&models_list,))?;

    // Load and execute the model file using importlib.util
    let importlib_util = py.import("importlib.util")?;
    let file_path_str = file_path.to_string_lossy().to_string();
    let spec = importlib_util.call_method1("spec_from_file_location", ("model", &file_path_str))?;

    if spec.is_none() {
        return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
            "Could not load Python model file: {}",
            file_path.display()
        )));
    }

    let loader = spec.getattr("loader")?;
    if loader.is_none() {
        return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
            "No loader for Python model file: {}",
            file_path.display()
        )));
    }

    let module = importlib_util.call_method1("module_from_spec", (&spec,))?;
    loader.call_method1("exec_module", (&module,))?;

    // Iterate registered models and call each function
    let builtins = py.import("builtins")?;
    let list_fn = builtins.getattr("list")?;
    let items = registered_models.call_method0("items")?;
    let items_list: Vec<(String, Bound<'_, PyAny>)> = list_fn.call1((items,))?.extract()?;

    let mut results = Vec::new();
    for (name, func) in &items_list {
        // Reset query log so each model only records its own queries
        project_context.setattr("_queries", PyList::empty(py))?;

        let sql_result = func.call1((&project_context,))?;

        // Validate return type
        let sql: String = sql_result.extract().map_err(|_| {
            let type_name = sql_result
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "<unknown>".to_string());
            PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
                "Model '{}' must return a string, got {}",
                name, type_name
            ))
        })?;

        // Extract queries
        let queries_list: Vec<Bound<'_, PyAny>> = project_context.getattr("_queries")?.extract()?;
        let mut queries = Vec::new();
        for q in &queries_list {
            let kind: String = q.get_item("kind")?.extract()?;
            let tag: Option<String> =
                q.get_item("tag")
                    .ok()
                    .and_then(|v| if v.is_none() { None } else { v.extract().ok() });
            let directory: Option<String> = q.get_item("directory").ok().and_then(|v| {
                if v.is_none() {
                    None
                } else {
                    v.extract().ok()
                }
            });
            queries.push(PythonModelQuery {
                kind,
                tag,
                directory,
            });
        }

        results.push(PythonModelOutput {
            name: name.clone(),
            sql,
            queries,
        });
    }

    Ok(results)
}

/// Ensure the smelt Python SDK is importable by adding its path to `sys.path`.
///
/// This should be called once before any `run_python_model_file` calls.
/// The `sdk_path` should point to the directory containing the `smelt` Python package
/// (e.g., `<project>/python/`).
pub fn ensure_sdk_on_path(py: Python<'_>, sdk_path: &Path) -> PyResult<()> {
    let sys = py.import("sys")?;
    let sys_path = sys.getattr("path")?;
    let sdk_str = sdk_path.to_string_lossy().to_string();
    let contains: bool = sys_path
        .call_method1("__contains__", (&sdk_str,))?
        .extract()?;
    if !contains {
        sys_path.call_method1("insert", (0, &sdk_str))?;
    }
    Ok(())
}
