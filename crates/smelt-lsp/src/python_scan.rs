//! Python model scanning for LSP awareness.
//!
//! This discovers Python models and runs them via subprocess to get their SQL,
//! so they can be registered in Salsa and be valid ref targets.
//!
//! Features:
//! - Content-hash caching to avoid re-executing unchanged Python files
//! - Error collection for surfacing as LSP diagnostics
//! - Single-file re-execution for file-watch updates

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(feature = "python"))]
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A Python model discovered and executed.
pub struct PythonModel {
    pub name: String,
    pub sql: String,
    pub source_path: PathBuf,
    /// Line number of the `@model` decorator (0-indexed), for goto-definition.
    pub decorator_line: u32,
}

/// An error from Python model execution.
pub struct PythonModelError {
    pub source_path: PathBuf,
    pub message: String,
    /// Line number in the .py file (1-indexed), if extractable from traceback.
    pub line: Option<u32>,
}

/// Result of scanning Python models — both successes and errors.
pub struct PythonScanResult {
    pub models: Vec<PythonModel>,
    pub errors: Vec<PythonModelError>,
}

#[derive(Deserialize)]
struct PythonModelOutput {
    name: String,
    sql: String,
}

/// On-disk cache for Python model results.
#[derive(Serialize, Deserialize, Default)]
pub struct PythonModelCache {
    entries: HashMap<PathBuf, CacheEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct CacheEntry {
    content_hash: String,
    models: Vec<CachedModel>,
    #[allow(dead_code)]
    timestamp: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct CachedModel {
    name: String,
    sql: String,
    decorator_line: u32,
}

impl PythonModelCache {
    /// Load cache from disk, or return empty cache if file doesn't exist or is invalid.
    pub fn load(project_dir: &Path) -> Self {
        let cache_path = Self::cache_path(project_dir);
        match std::fs::read_to_string(&cache_path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save cache to disk.
    pub fn save(&self, project_dir: &Path) {
        let cache_path = Self::cache_path(project_dir);
        if let Some(parent) = cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&cache_path, json);
        }
    }

    fn cache_path(project_dir: &Path) -> PathBuf {
        project_dir.join(".smelt").join("python_cache.json")
    }

    /// Look up a cached result by file path and content hash.
    fn get(&self, file_path: &Path, content_hash: &str) -> Option<&CacheEntry> {
        self.entries
            .get(file_path)
            .filter(|entry| entry.content_hash == content_hash)
    }

    /// Store a result in the cache.
    fn put(&mut self, file_path: PathBuf, content_hash: String, models: Vec<CachedModel>) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        self.entries.insert(
            file_path,
            CacheEntry {
                content_hash,
                models,
                timestamp,
            },
        );
    }
}

/// Compute SHA-256 hash of content.
fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Check if a Python file contains `@model` decorators.
pub fn has_model_decorator(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "@model" || trimmed.starts_with("@model(")
    })
}

/// Build a map of model function name → decorator line (0-indexed).
/// Scans for `@model` decorators followed by `def func_name(...)`.
fn build_decorator_map(content: &str) -> HashMap<String, u32> {
    let lines: Vec<&str> = content.lines().collect();
    let mut map = HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed == "@model" || trimmed.starts_with("@model(") {
            let decorator_line = i as u32;
            // Scan forward for the `def` line
            let mut j = i + 1;
            while j < lines.len() {
                let def_trimmed = lines[j].trim();
                if def_trimmed.is_empty() {
                    j += 1;
                    continue;
                }
                if def_trimmed.starts_with("def ") {
                    if let Some(name) = def_trimmed
                        .strip_prefix("def ")
                        .and_then(|rest| rest.split('(').next())
                        .map(|n| n.trim().to_string())
                    {
                        map.insert(name, decorator_line);
                    }
                }
                break;
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    map
}

/// Find the Python interpreter (python3 or python).
#[cfg(not(feature = "python"))]
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
#[cfg(not(feature = "python"))]
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

/// Try to extract a line number from a Python traceback string.
/// Looks for patterns like `File "...", line N`.
#[cfg(not(feature = "python"))]
fn extract_line_from_traceback(stderr: &str) -> Option<u32> {
    // Find the last "line N" in the traceback (most specific frame)
    let mut last_line = None;
    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("File ") {
            if let Some(pos) = trimmed.find(", line ") {
                let after = &trimmed[pos + 7..];
                if let Some(num_str) = after.split([',', '\n', ' ']).next() {
                    if let Ok(n) = num_str.parse::<u32>() {
                        last_line = Some(n);
                    }
                }
            }
        }
    }
    last_line
}

/// Execute a single Python file via subprocess and return results + errors.
#[cfg(not(feature = "python"))]
fn execute_python_file(
    python: &str,
    file_path: &Path,
    context_json: &str,
    sdk_path: &Path,
) -> (Vec<PythonModelOutput>, Option<PythonModelError>) {
    let pythonpath = build_pythonpath(sdk_path, file_path);
    let output = match Command::new(python)
        .arg("-m")
        .arg("smelt.runner")
        .arg(file_path)
        .arg(context_json)
        .env("PYTHONPATH", pythonpath)
        .output()
    {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let line = extract_line_from_traceback(&stderr);
            return (
                Vec::new(),
                Some(PythonModelError {
                    source_path: file_path.to_path_buf(),
                    message: format!("Python model execution failed: {}", stderr.trim()),
                    line,
                }),
            );
        }
        Err(e) => {
            return (
                Vec::new(),
                Some(PythonModelError {
                    source_path: file_path.to_path_buf(),
                    message: format!("Failed to run Python: {}", e),
                    line: None,
                }),
            );
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<Vec<PythonModelOutput>>(&stdout) {
        Ok(outputs) => (outputs, None),
        Err(e) => (
            Vec::new(),
            Some(PythonModelError {
                source_path: file_path.to_path_buf(),
                message: format!("Invalid JSON from Python model: {}", e),
                line: None,
            }),
        ),
    }
}

/// Execute a single Python file via embedded PyO3 interpreter.
#[cfg(feature = "python")]
fn execute_python_file(
    _python: &str,
    file_path: &Path,
    context_json: &str,
    sdk_path: &Path,
) -> (Vec<PythonModelOutput>, Option<PythonModelError>) {
    use pyo3::prelude::*;
    use pyo3::types::PyTracebackMethods;

    let result: Result<Vec<PythonModelOutput>, String> = Python::with_gil(|py| {
        smelt_core::python_models::ensure_sdk_on_path(py, sdk_path)
            .map_err(|e| format!("Failed to set up Python SDK path: {}", e))?;

        let outputs = smelt_core::python_models::run_python_model_file(py, file_path, context_json)
            .map_err(|e| {
                let tb = e
                    .traceback(py)
                    .map(|tb| tb.format().unwrap_or_default())
                    .unwrap_or_default();
                format!("{}{}", tb, e)
            })?;

        Ok(outputs
            .into_iter()
            .map(|o| PythonModelOutput {
                name: o.name,
                sql: o.sql,
            })
            .collect())
    });

    match result {
        Ok(outputs) => (outputs, None),
        Err(msg) => (
            Vec::new(),
            Some(PythonModelError {
                source_path: file_path.to_path_buf(),
                message: format!("Python model execution failed: {}", msg),
                line: None,
            }),
        ),
    }
}

/// Scan a models directory for Python files and execute them to get SQL.
/// Uses content-hash caching to avoid re-executing unchanged files.
/// Returns discovered models and any errors encountered.
pub fn discover_python_models(
    models_path: &Path,
    project_dir: &Path,
    cache: &mut PythonModelCache,
) -> PythonScanResult {
    // With PyO3, we don't need a separate Python interpreter.
    #[cfg(feature = "python")]
    let python = String::new();
    #[cfg(not(feature = "python"))]
    let python = match find_python() {
        Some(p) => p,
        None => {
            return PythonScanResult {
                models: Vec::new(),
                errors: Vec::new(),
            }
        }
    };

    let sdk_path = match find_python_sdk(project_dir) {
        Some(p) => p,
        None => {
            return PythonScanResult {
                models: Vec::new(),
                errors: Vec::new(),
            }
        }
    };

    let mut python_files: Vec<(PathBuf, String)> = Vec::new();

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
                python_files.push((path.to_path_buf(), content));
            }
        }
    }

    if python_files.is_empty() {
        return PythonScanResult {
            models: Vec::new(),
            errors: Vec::new(),
        };
    }

    let context_json = r#"{"models": []}"#;
    let mut models = Vec::new();
    let mut errors = Vec::new();

    for (file_path, content) in &python_files {
        let hash = content_hash(content);
        let decorator_map = build_decorator_map(content);

        // Check cache first
        if let Some(cached) = cache.get(file_path, &hash) {
            for cm in &cached.models {
                models.push(PythonModel {
                    name: cm.name.clone(),
                    sql: cm.sql.clone(),
                    source_path: file_path.clone(),
                    decorator_line: cm.decorator_line,
                });
            }
            continue;
        }

        // Cache miss — execute
        let (outputs, error) = execute_python_file(&python, file_path, context_json, &sdk_path);

        if let Some(err) = error {
            errors.push(err);
        } else {
            // Cache successful results
            let cached_models: Vec<CachedModel> = outputs
                .iter()
                .map(|o| CachedModel {
                    name: o.name.clone(),
                    sql: o.sql.clone(),
                    decorator_line: decorator_map.get(&o.name).copied().unwrap_or(0),
                })
                .collect();
            cache.put(file_path.clone(), hash, cached_models);
        }

        for out in outputs {
            let decorator_line = decorator_map.get(&out.name).copied().unwrap_or(0);
            models.push(PythonModel {
                name: out.name,
                sql: out.sql,
                source_path: file_path.clone(),
                decorator_line,
            });
        }
    }

    cache.save(project_dir);

    PythonScanResult { models, errors }
}

/// Execute a single Python model file and return results.
/// Updates the cache if execution succeeds.
pub fn execute_single_python_file(
    file_path: &Path,
    project_dir: &Path,
    cache: &mut PythonModelCache,
) -> PythonScanResult {
    #[cfg(feature = "python")]
    let python = String::new();
    #[cfg(not(feature = "python"))]
    let python = match find_python() {
        Some(p) => p,
        None => {
            return PythonScanResult {
                models: Vec::new(),
                errors: vec![PythonModelError {
                    source_path: file_path.to_path_buf(),
                    message: "Python interpreter not found".to_string(),
                    line: None,
                }],
            }
        }
    };

    let sdk_path = match find_python_sdk(project_dir) {
        Some(p) => p,
        None => {
            return PythonScanResult {
                models: Vec::new(),
                errors: vec![PythonModelError {
                    source_path: file_path.to_path_buf(),
                    message: "smelt Python SDK not found".to_string(),
                    line: None,
                }],
            }
        }
    };

    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            return PythonScanResult {
                models: Vec::new(),
                errors: vec![PythonModelError {
                    source_path: file_path.to_path_buf(),
                    message: format!("Failed to read file: {}", e),
                    line: None,
                }],
            }
        }
    };

    if !has_model_decorator(&content) {
        return PythonScanResult {
            models: Vec::new(),
            errors: Vec::new(),
        };
    }

    let hash = content_hash(&content);
    let decorator_map = build_decorator_map(&content);

    // Check cache
    if let Some(cached) = cache.get(file_path, &hash) {
        let models = cached
            .models
            .iter()
            .map(|cm| PythonModel {
                name: cm.name.clone(),
                sql: cm.sql.clone(),
                source_path: file_path.to_path_buf(),
                decorator_line: cm.decorator_line,
            })
            .collect();
        return PythonScanResult {
            models,
            errors: Vec::new(),
        };
    }

    let context_json = r#"{"models": []}"#;
    let (outputs, error) = execute_python_file(&python, file_path, context_json, &sdk_path);

    let mut models = Vec::new();
    let mut errors = Vec::new();

    if let Some(err) = error {
        errors.push(err);
    } else {
        let cached_models: Vec<CachedModel> = outputs
            .iter()
            .map(|o| CachedModel {
                name: o.name.clone(),
                sql: o.sql.clone(),
                decorator_line: decorator_map.get(&o.name).copied().unwrap_or(0),
            })
            .collect();
        cache.put(file_path.to_path_buf(), hash, cached_models);
        cache.save(project_dir);
    }

    for out in outputs {
        let decorator_line = decorator_map.get(&out.name).copied().unwrap_or(0);
        models.push(PythonModel {
            name: out.name,
            sql: out.sql,
            source_path: file_path.to_path_buf(),
            decorator_line,
        });
    }

    PythonScanResult { models, errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_model_decorator() {
        assert!(has_model_decorator("@model\ndef foo(project):\n    pass"));
        assert!(has_model_decorator("  @model\ndef foo(project):\n    pass"));
        assert!(has_model_decorator("@model()\ndef foo(project):\n    pass"));
        assert!(!has_model_decorator("def foo():\n    pass"));
        assert!(!has_model_decorator("# @model\ndef foo():\n    pass"));
    }

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        assert_eq!(h1, h2);

        let h3 = content_hash("hello world!");
        assert_ne!(h1, h3);
    }

    #[test]
    #[cfg(not(feature = "python"))]
    fn test_extract_line_from_traceback() {
        let traceback = r#"Traceback (most recent call last):
  File "/path/to/model.py", line 42, in <module>
    result = foo()
  File "/path/to/model.py", line 10, in foo
    raise ValueError("bad")
ValueError: bad"#;
        assert_eq!(extract_line_from_traceback(traceback), Some(10));
    }

    #[test]
    #[cfg(not(feature = "python"))]
    fn test_extract_line_no_traceback() {
        assert_eq!(extract_line_from_traceback("some error message"), None);
    }

    #[test]
    fn test_build_decorator_map() {
        let content = r#"from smelt import model

@model
def combined_events(project):
    return "SELECT 1"

@model
def other_model(project):
    return "SELECT 2"
"#;
        let map = build_decorator_map(content);
        assert_eq!(map.get("combined_events"), Some(&2)); // 0-indexed line
        assert_eq!(map.get("other_model"), Some(&6));
    }

    #[test]
    fn test_cache_put_and_get() {
        let mut cache = PythonModelCache::default();
        let path = PathBuf::from("/test/model.py");
        let hash = "abc123".to_string();

        assert!(cache.get(&path, &hash).is_none());

        cache.put(
            path.clone(),
            hash.clone(),
            vec![CachedModel {
                name: "test_model".to_string(),
                sql: "SELECT 1".to_string(),
                decorator_line: 0,
            }],
        );

        let entry = cache.get(&path, &hash).unwrap();
        assert_eq!(entry.models.len(), 1);
        assert_eq!(entry.models[0].name, "test_model");

        // Different hash returns None (invalidated)
        assert!(cache.get(&path, "different_hash").is_none());
    }
}
