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
use smelt_core::python_utils;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(feature = "python"))]
use std::process::{Command, Stdio};
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
    #[serde(default)]
    context_hash: String,
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
            // intentionally ignored: cache is best-effort; a missing or
            // unreadable cache file means a cold start, not an error.
            Err(_) => Self::default(),
        }
    }

    /// Save cache to disk.
    pub fn save(&self, project_dir: &Path) {
        let cache_path = Self::cache_path(project_dir);
        if let Some(parent) = cache_path.parent() {
            // intentionally ignored: cache dir creation is best-effort.
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            // intentionally ignored: cache write failure is non-fatal.
            let _ = std::fs::write(&cache_path, json);
        }
    }

    fn cache_path(project_dir: &Path) -> PathBuf {
        project_dir.join(".smelt").join("python_cache.json")
    }

    /// Look up a cached result by file path, content hash, and context hash.
    fn get(&self, file_path: &Path, content_hash: &str, context_hash: &str) -> Option<&CacheEntry> {
        self.entries.get(file_path).filter(|entry| {
            entry.content_hash == content_hash && entry.context_hash == context_hash
        })
    }

    /// Store a result in the cache.
    fn put(
        &mut self,
        file_path: PathBuf,
        content_hash: String,
        context_hash: String,
        models: Vec<CachedModel>,
    ) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        self.entries.insert(
            file_path,
            CacheEntry {
                content_hash,
                context_hash,
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

// Shared helpers re-exported from smelt_core::python_utils:
// - has_model_decorator, build_decorator_map, find_python, find_python_sdk,
//   build_pythonpath, extract_line_from_traceback

/// Execute a single Python file via subprocess and return results + errors.
#[cfg(not(feature = "python"))]
fn execute_python_file(
    python: &str,
    file_path: &Path,
    context_json: &str,
    sdk_path: &Path,
) -> (Vec<PythonModelOutput>, Option<PythonModelError>) {
    let pythonpath = python_utils::build_pythonpath(sdk_path, file_path);
    // Pass context via stdin to avoid OS argument size limits (E2BIG).
    let mut child = match Command::new(python)
        .arg("-m")
        .arg("smelt.runner")
        .arg(file_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONPATH", pythonpath)
        .spawn()
    {
        Ok(c) => c,
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
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        // intentionally ignored: if stdin write fails, the child process will
        // terminate with its own error and child.wait_with_output() catches it.
        let _ = stdin.write_all(context_json.as_bytes());
    }
    let output = match child.wait_with_output() {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let line = python_utils::extract_line_from_traceback(&stderr);
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

    let result: Result<Vec<PythonModelOutput>, String> = Python::attach(|py| {
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
    context_json: &str,
) -> PythonScanResult {
    // With PyO3, we don't need a separate Python interpreter.
    #[cfg(feature = "python")]
    let python = String::new();
    #[cfg(not(feature = "python"))]
    let python = match python_utils::find_python(None) {
        Some(p) => p,
        None => {
            return PythonScanResult {
                models: Vec::new(),
                errors: Vec::new(),
            }
        }
    };

    let sdk_path = match python_utils::find_python_sdk(project_dir) {
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
            if python_utils::has_model_decorator(&content) {
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

    let ctx_hash = content_hash(context_json);
    let mut models = Vec::new();
    let mut errors = Vec::new();

    for (file_path, content) in &python_files {
        let hash = content_hash(content);
        let decorator_map = python_utils::build_decorator_map(content);

        // Check cache first
        if let Some(cached) = cache.get(file_path, &hash, &ctx_hash) {
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
            cache.put(file_path.clone(), hash, ctx_hash.clone(), cached_models);
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
    context_json: &str,
) -> PythonScanResult {
    #[cfg(feature = "python")]
    let python = String::new();
    #[cfg(not(feature = "python"))]
    let python = match python_utils::find_python(None) {
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

    let sdk_path = match python_utils::find_python_sdk(project_dir) {
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

    if !python_utils::has_model_decorator(&content) {
        return PythonScanResult {
            models: Vec::new(),
            errors: Vec::new(),
        };
    }

    let hash = content_hash(&content);
    let ctx_hash = content_hash(context_json);
    let decorator_map = python_utils::build_decorator_map(&content);

    // Check cache
    if let Some(cached) = cache.get(file_path, &hash, &ctx_hash) {
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
        cache.put(file_path.to_path_buf(), hash, ctx_hash, cached_models);
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
    fn test_content_hash_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        assert_eq!(h1, h2);

        let h3 = content_hash("hello world!");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_cache_put_and_get() {
        let mut cache = PythonModelCache::default();
        let path = PathBuf::from("/test/model.py");
        let hash = "abc123".to_string();
        let ctx_hash = "ctx456".to_string();

        assert!(cache.get(&path, &hash, &ctx_hash).is_none());

        cache.put(
            path.clone(),
            hash.clone(),
            ctx_hash.clone(),
            vec![CachedModel {
                name: "test_model".to_string(),
                sql: "SELECT 1".to_string(),
                decorator_line: 0,
            }],
        );

        let entry = cache.get(&path, &hash, &ctx_hash).unwrap();
        assert_eq!(entry.models.len(), 1);
        assert_eq!(entry.models[0].name, "test_model");

        // Different content hash returns None (invalidated)
        assert!(cache.get(&path, "different_hash", &ctx_hash).is_none());

        // Different context hash returns None (invalidated when model list changes)
        assert!(cache.get(&path, &hash, "different_ctx").is_none());
    }
}
