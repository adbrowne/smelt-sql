//! Shared PyO3-based Python model execution.
//!
//! When the `python` feature is enabled, this module provides in-process
//! execution of Python model files via PyO3, eliminating subprocess overhead.
//! Used by both CLI and LSP for Python model discovery.

use pyo3::prelude::*;
use pyo3::types::PyList;
use std::path::Path;
use std::sync::{Condvar, Mutex};

/// Serialises the clear/exec/collect critical section of [`run_python_model_file`].
///
/// The embedded interpreter is process-global, and so is the registry the
/// `@model` decorator writes to (`smelt.core._registered_models`). The GIL alone
/// does **not** protect that section: CPython drops the GIL on every I/O call a
/// model makes, so two concurrent `run_python_model_file` calls interleave —
/// A clears, A execs, B clears (dropping A's registration and adding its own),
/// then A iterates the registry and runs *B's* model. The observable symptoms
/// are a model file returning another file's models, and an error attributed to
/// one file carrying another file's traceback (issue #189).
///
/// Acquire this **with the GIL released** (see [`lock_registry`]): a thread that
/// held the GIL while blocking here would deadlock against the lock holder
/// waiting to re-acquire the GIL.
static REGISTRY_HELD: Mutex<bool> = Mutex::new(false);
static REGISTRY_CV: Condvar = Condvar::new();

/// RAII release for the registry lock acquired by [`lock_registry`].
struct RegistryGuard;

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        let mut held = REGISTRY_HELD.lock().unwrap_or_else(|e| e.into_inner());
        *held = false;
        REGISTRY_CV.notify_one();
    }
}

/// Acquire [`REGISTRY_HELD`] with the GIL released.
///
/// A plain `MutexGuard` cannot cross `Python::detach` (it is not `Send`), so the
/// wait is expressed as a condvar gate whose guard stays inside the closure and
/// the ownership flag is handed back through the `RegistryGuard`.
fn lock_registry(py: Python<'_>) -> RegistryGuard {
    py.detach(|| {
        let mut held = REGISTRY_HELD.lock().unwrap_or_else(|e| e.into_inner());
        while *held {
            held = REGISTRY_CV.wait(held).unwrap_or_else(|e| e.into_inner());
        }
        *held = true;
    });
    RegistryGuard
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Python's global `_registered_models` dict is shared across tests, so we
    /// must serialize all tests that touch `run_python_model_file`.
    static PYTHON_LOCK: Mutex<()> = Mutex::new(());

    fn find_test_sdk() -> Option<PathBuf> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir.parent()?.parent()?;
        let sdk_path = repo_root.join("python");
        if sdk_path.join("smelt").is_dir() {
            Some(sdk_path)
        } else {
            None
        }
    }

    #[test]
    fn test_run_simple_model() {
        let _lock = PYTHON_LOCK.lock().unwrap();
        let sdk_path = match find_test_sdk() {
            Some(p) => p,
            None => return, // skip if SDK not found
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let model_path = tmp.path().join("test_model.py");
        std::fs::write(
            &model_path,
            r#"from smelt.core import model, _registered_models

@model
def simple_model(project):
    return "SELECT 1 as id"
"#,
        )
        .unwrap();

        Python::attach(|py| {
            ensure_sdk_on_path(py, &sdk_path).unwrap();
            let results = run_python_model_file(py, &model_path, r#"{"models": []}"#).unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].name, "simple_model");
            assert_eq!(results[0].sql, "SELECT 1 as id");
            assert!(results[0].queries.is_empty());
        });
    }

    #[test]
    fn test_run_model_with_find_models() {
        let _lock = PYTHON_LOCK.lock().unwrap();
        let sdk_path = match find_test_sdk() {
            Some(p) => p,
            None => return,
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let model_path = tmp.path().join("query_model.py");
        std::fs::write(
            &model_path,
            r#"from smelt.core import model

@model
def query_model(project):
    children = project.find_models(tag="src")
    names = [m.name for m in children]
    return "SELECT " + ", ".join(names) if names else "SELECT 1"
"#,
        )
        .unwrap();

        let context =
            r#"{"models": [{"name": "foo", "tags": ["src"]}, {"name": "bar", "tags": []}]}"#;

        Python::attach(|py| {
            ensure_sdk_on_path(py, &sdk_path).unwrap();
            let results = run_python_model_file(py, &model_path, context).unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].name, "query_model");
            assert!(results[0].sql.contains("foo"));
            assert_eq!(results[0].queries.len(), 1);
            assert_eq!(results[0].queries[0].kind, "find_models");
            assert_eq!(results[0].queries[0].tag.as_deref(), Some("src"));
        });
    }

    #[test]
    fn test_run_multiple_models_one_file() {
        let _lock = PYTHON_LOCK.lock().unwrap();
        let sdk_path = match find_test_sdk() {
            Some(p) => p,
            None => return,
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let model_path = tmp.path().join("multi.py");
        std::fs::write(
            &model_path,
            r#"from smelt.core import model

@model
def model_a(project):
    return "SELECT 1"

@model
def model_b(project):
    return "SELECT 2"
"#,
        )
        .unwrap();

        Python::attach(|py| {
            ensure_sdk_on_path(py, &sdk_path).unwrap();
            let results = run_python_model_file(py, &model_path, r#"{"models": []}"#).unwrap();
            assert_eq!(results.len(), 2);
            let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
            assert!(names.contains(&"model_a"));
            assert!(names.contains(&"model_b"));
        });
    }

    #[test]
    fn test_non_string_return_errors() {
        let _lock = PYTHON_LOCK.lock().unwrap();
        let sdk_path = match find_test_sdk() {
            Some(p) => p,
            None => return,
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let model_path = tmp.path().join("bad_return.py");
        std::fs::write(
            &model_path,
            r#"from smelt.core import model

@model
def bad_model(project):
    return 42
"#,
        )
        .unwrap();

        Python::attach(|py| {
            ensure_sdk_on_path(py, &sdk_path).unwrap();
            let result = run_python_model_file(py, &model_path, r#"{"models": []}"#);
            assert!(result.is_err());
        });
    }
}
