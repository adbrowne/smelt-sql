//! Shared Python model utilities used by both LSP and CLI.
//!
//! Contains helpers for finding the Python interpreter, locating the smelt SDK,
//! building PYTHONPATH, and scanning for `@model` decorators.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Find the Python interpreter to use.
/// Resolution order: SMELT_PYTHON env var → config_python parameter → python3 → python
pub fn find_python(config_python: Option<&str>) -> Option<String> {
    // 1. SMELT_PYTHON env var
    if let Ok(python) = std::env::var("SMELT_PYTHON") {
        return Some(python);
    }

    // 2. Config python field
    if let Some(python) = config_python {
        return Some(python.to_string());
    }

    // 3. Try python3
    if Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some("python3".to_string());
    }

    // 4. Try python
    if Command::new("python")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some("python".to_string());
    }

    None
}

/// Find the Python SDK path.
/// Resolution order: SMELT_PYTHON_SDK env var → project_dir/python/ → walk up from project_dir
pub fn find_python_sdk(project_dir: &Path) -> Option<PathBuf> {
    // 1. SMELT_PYTHON_SDK env var
    if let Ok(sdk_path) = std::env::var("SMELT_PYTHON_SDK") {
        let path = PathBuf::from(sdk_path);
        if path.join("smelt").is_dir() {
            return Some(path);
        }
    }

    // 2. project_dir/python/
    let project_sdk = project_dir.join("python");
    if project_sdk.join("smelt").is_dir() {
        return Some(project_sdk);
    }

    // 3. Walk up from project_dir (for monorepo/workspace layouts)
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
/// to any existing PYTHONPATH. Uses platform-appropriate path separator.
pub fn build_pythonpath(sdk_path: &Path, file_path: &Path) -> std::ffi::OsString {
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

/// Check if a Python file contains `@model` decorators.
pub fn has_model_decorator(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "@model" || trimmed.starts_with("@model(")
    })
}

/// Return 0-indexed line numbers of all `@model` decorators in a Python file.
pub fn scan_for_model_decorators(content: &str) -> Vec<u32> {
    content
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let trimmed = line.trim();
            if trimmed == "@model" || trimmed.starts_with("@model(") {
                Some(i as u32)
            } else {
                None
            }
        })
        .collect()
}

/// Build a map from function name to the 0-indexed line number of its `@model` decorator.
/// Scans for `@model` (or `@model(...)`) followed by `def <name>`.
pub fn build_decorator_map(content: &str) -> HashMap<String, u32> {
    let lines: Vec<&str> = content.lines().collect();
    let mut map = HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed == "@model" || trimmed.starts_with("@model(") {
            let decorator_line = i as u32;
            // Scan forward for the `def` line, skipping blank lines
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

/// Try to extract a line number from a Python traceback string.
/// Looks for patterns like `File "...", line N` and returns the last match (most specific frame).
pub fn extract_line_from_traceback(stderr: &str) -> Option<u32> {
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

/// Data passed to Python models as project context.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ProjectContextData {
    pub models: Vec<ProjectModelInfo>,
}

/// Model info visible to Python's `project.find_models()`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ProjectModelInfo {
    pub name: String,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
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
    fn test_scan_for_model_decorators() {
        let content = r#"
from smelt import model

@model
def combined_events(project):
    return "SELECT 1"

def helper():
    pass

@model
def another_model(project):
    return "SELECT 2"
"#;
        let lines = scan_for_model_decorators(content);
        assert_eq!(lines, vec![3, 10]); // 0-indexed
    }

    #[test]
    fn test_scan_no_decorators() {
        let content = "def foo(): pass\n";
        let lines = scan_for_model_decorators(content);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_scan_ignores_non_model_decorators() {
        let content = r#"
@other_decorator
def foo():
    pass

@model
def bar(project):
    return "SELECT 1"
"#;
        let lines = scan_for_model_decorators(content);
        assert_eq!(lines, vec![5]); // 0-indexed
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
        assert_eq!(map.get("combined_events"), Some(&2)); // 0-indexed
        assert_eq!(map.get("other_model"), Some(&6));
    }

    #[test]
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
    fn test_extract_line_no_traceback() {
        assert_eq!(extract_line_from_traceback("some error message"), None);
    }
}
