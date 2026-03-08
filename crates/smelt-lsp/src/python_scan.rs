//! Minimal Python model scanning for LSP awareness.
//!
//! This discovers Python models and runs them via subprocess to get their SQL,
//! so they can be registered in Salsa and be valid ref targets.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A Python model discovered and executed.
pub struct PythonModel {
    #[allow(dead_code)]
    pub name: String,
    pub sql: String,
    pub source_path: PathBuf,
}

#[derive(Deserialize)]
struct PythonModelOutput {
    name: String,
    sql: String,
}

/// Check if a Python file contains `@model` decorators.
fn has_model_decorator(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "@model" || trimmed.starts_with("@model(")
    })
}

/// Find the Python interpreter (python3 or python).
fn find_python() -> Option<String> {
    if let Ok(python) = std::env::var("SMELT_PYTHON") {
        return Some(python);
    }
    for name in &["python3", "python"] {
        if Command::new(name)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Find the Python SDK path by walking up from project_dir.
fn find_python_sdk(project_dir: &Path) -> Option<PathBuf> {
    if let Ok(sdk_path) = std::env::var("SMELT_PYTHON_SDK") {
        let path = PathBuf::from(sdk_path);
        if path.join("smelt").is_dir() {
            return Some(path);
        }
    }

    let mut current = project_dir.to_path_buf();
    for _ in 0..5 {
        let candidate = current.join("python");
        if candidate.join("smelt").is_dir() {
            return Some(candidate);
        }
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }
    None
}

/// Build PYTHONPATH by prepending SDK path and model file's parent directory
/// to any existing PYTHONPATH.
fn build_pythonpath(sdk_path: &Path, file_path: &Path) -> std::ffi::OsString {
    let mut paths: Vec<PathBuf> = vec![sdk_path.to_path_buf()];
    if let Some(parent) = file_path.parent() {
        paths.push(parent.to_path_buf());
    }
    if let Ok(existing) = std::env::var("PYTHONPATH") {
        for p in std::env::split_paths(&existing) {
            paths.push(p);
        }
    }
    std::env::join_paths(paths).unwrap_or_else(|_| sdk_path.as_os_str().to_os_string())
}

/// Scan a models directory for Python files and execute them to get SQL.
/// Returns a list of discovered Python models, or empty if Python is unavailable.
pub fn discover_python_models(models_path: &Path, project_dir: &Path) -> Vec<PythonModel> {
    let python = match find_python() {
        Some(p) => p,
        None => return Vec::new(),
    };

    let sdk_path = match find_python_sdk(project_dir) {
        Some(p) => p,
        None => return Vec::new(),
    };

    let mut python_files = Vec::new();

    // Scan for .py files with @model decorators
    for entry in walkdir::WalkDir::new(models_path)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("py") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(path) {
            if has_model_decorator(&content) {
                python_files.push(path.to_path_buf());
            }
        }
    }

    if python_files.is_empty() {
        return Vec::new();
    }

    // Build minimal project context (no model info yet for LSP init)
    let context_json = r#"{"models": []}"#;

    let mut models = Vec::new();
    for file_path in &python_files {
        let pythonpath = build_pythonpath(&sdk_path, file_path);
        let output = match Command::new(&python)
            .arg("-m")
            .arg("smelt.runner")
            .arg(file_path)
            .arg(context_json)
            .env("PYTHONPATH", pythonpath)
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => continue,
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Ok(outputs) = serde_json::from_str::<Vec<PythonModelOutput>>(&stdout) {
            for out in outputs {
                models.push(PythonModel {
                    name: out.name,
                    sql: out.sql,
                    source_path: file_path.clone(),
                });
            }
        }
    }

    models
}
