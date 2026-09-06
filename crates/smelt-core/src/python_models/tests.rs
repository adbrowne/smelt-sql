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

    let context = r#"{"models": [{"name": "foo", "tags": ["src"]}, {"name": "bar", "tags": []}]}"#;

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
