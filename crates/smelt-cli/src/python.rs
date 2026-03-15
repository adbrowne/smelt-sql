//! Python model discovery and execution.
//!
//! Python models are the "escape hatch" — deliberately low-level, for the ~5% of cases
//! where you need programmatic model generation (e.g., dynamically union all models with a tag).
//! Python models return SQL strings that get parsed by the existing smelt parser.

#[cfg(not(feature = "python"))]
use anyhow::Context;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;
use crate::discovery::{ModelFile, ModelKind};
use crate::errors::CliError;
use crate::metadata::{extract_file_metadata, FileMetadata};

/// Data passed to Python models as project context.
#[derive(Debug, Serialize)]
struct ProjectContextData {
    models: Vec<ProjectModelInfo>,
}

/// Model info visible to Python's `project.find_models()`.
#[derive(Debug, Serialize)]
struct ProjectModelInfo {
    name: String,
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    directory: Option<String>,
}

/// Output from a single Python model function.
#[derive(Debug, Deserialize)]
struct PythonModelOutput {
    name: String,
    sql: String,
    #[serde(default)]
    queries: Vec<PythonQuery>,
}

/// A query recorded by ProjectContext (for fixed-point validation).
#[derive(Debug, Deserialize)]
struct PythonQuery {
    kind: String,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    directory: Option<String>,
}

/// Check if a Python file contains `@model` decorators.
/// Returns the line numbers of `@model` decorators found.
pub fn scan_for_model_decorators(content: &str) -> Vec<usize> {
    content
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let trimmed = line.trim();
            if trimmed == "@model" || trimmed.starts_with("@model(") {
                Some(i + 1) // 1-indexed
            } else {
                None
            }
        })
        .collect()
}

/// Find the Python SDK path.
/// Resolution order: SMELT_PYTHON_SDK env var → project_dir/python/ → walk up from project_dir
pub fn find_python_sdk(project_dir: &Path) -> Result<PathBuf> {
    // 1. SMELT_PYTHON_SDK env var
    if let Ok(sdk_path) = std::env::var("SMELT_PYTHON_SDK") {
        let path = PathBuf::from(sdk_path);
        if path.join("smelt").is_dir() {
            return Ok(path);
        }
    }

    // 2. project_dir/python/
    let project_sdk = project_dir.join("python");
    if project_sdk.join("smelt").is_dir() {
        return Ok(project_sdk);
    }

    // 3. Walk up from project_dir (for monorepo/workspace layouts)
    let mut current = project_dir.to_path_buf();
    for _ in 0..5 {
        let candidate = current.join("python");
        if candidate.join("smelt").is_dir() {
            return Ok(candidate);
        }
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    Err(anyhow!(
        "smelt Python SDK not found. Expected a 'python/smelt/' directory in or above the project root.\n\
         Hint: Set SMELT_PYTHON_SDK to the directory containing the smelt Python package."
    ))
}

/// Find the Python interpreter to use.
/// Resolution order: SMELT_PYTHON env var → config python field → python3 → python
pub fn find_python(config_python: Option<&str>) -> Result<String> {
    // 1. SMELT_PYTHON env var
    if let Ok(python) = std::env::var("SMELT_PYTHON") {
        return Ok(python);
    }

    // 2. Config python field
    if let Some(python) = config_python {
        return Ok(python.to_string());
    }

    // 3. Try python3
    if Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok("python3".to_string());
    }

    // 4. Try python
    if Command::new("python")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok("python".to_string());
    }

    Err(CliError::PythonNotFound.into())
}

/// Execute a Python model file and return the generated SQL models (subprocess path).
#[cfg(not(feature = "python"))]
fn run_python_model(
    python: &str,
    file_path: &Path,
    project_context_json: &str,
    python_sdk_path: &Path,
) -> Result<Vec<PythonModelOutput>> {
    let pythonpath = build_pythonpath(python_sdk_path, file_path);
    let output = Command::new(python)
        .arg("-m")
        .arg("smelt.runner")
        .arg(file_path)
        .arg(project_context_json)
        .env("PYTHONPATH", pythonpath)
        .output()
        .with_context(|| format!("Failed to execute Python model: {}", file_path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CliError::PythonExecutionError {
            file: file_path.to_path_buf(),
            message: stderr.to_string(),
        }
        .into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).with_context(|| {
        format!(
            "Failed to parse Python model output from {}: {}",
            file_path.display(),
            stdout
        )
    })
}

/// Execute a Python model file via embedded PyO3 interpreter.
#[cfg(feature = "python")]
fn run_python_model(
    _python: &str,
    file_path: &Path,
    project_context_json: &str,
    python_sdk_path: &Path,
) -> Result<Vec<PythonModelOutput>> {
    use pyo3::prelude::*;
    use pyo3::types::PyTracebackMethods;

    Python::with_gil(|py| {
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
                CliError::PythonExecutionError {
                    file: file_path.to_path_buf(),
                    message: format!("{}{}", tb, e),
                }
            })?;

        Ok(outputs
            .into_iter()
            .map(|o| PythonModelOutput {
                name: o.name,
                sql: o.sql,
                queries: o
                    .queries
                    .into_iter()
                    .map(|q| PythonQuery {
                        kind: q.kind,
                        tag: q.tag,
                        directory: q.directory,
                    })
                    .collect(),
            })
            .collect())
    })
}

/// Build project context JSON from existing models.
fn build_project_context(
    sql_models: &[ModelFile],
    python_models: &[ModelFile],
    config: &Config,
) -> String {
    let mut models = Vec::new();

    for model in sql_models.iter().chain(python_models.iter()) {
        let tags = config.get_tags(&model.name, model.metadata.as_ref().map(|b| b.as_ref()));
        let directory = model
            .path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        models.push(ProjectModelInfo {
            name: model.name.clone(),
            tags,
            directory,
        });
    }

    let context = ProjectContextData { models };
    serde_json::to_string(&context).expect("Failed to serialize project context")
}

/// Build PYTHONPATH by prepending SDK path and model file's parent directory
/// to any existing PYTHONPATH. Uses platform-appropriate path separator.
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

/// Build a map from function name to the line number of its `@model` decorator.
/// Scans for `@model` (or `@model(...)`) followed by `def <name>`.
pub fn build_decorator_map(content: &str) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed == "@model" || trimmed.starts_with("@model(") {
            let decorator_line = i + 1; // 1-indexed
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

/// Discover and execute Python models.
///
/// Uses iterative discovery with fixed-point validation:
/// 1. Run all Python models with current project context
/// 2. If new models produced, rebuild context and re-run
/// 3. Stop when output stabilizes (or max rounds reached)
pub fn discover_python_models(
    python_files: &[(PathBuf, Vec<usize>, String)], // (path, decorator_lines, content)
    sql_models: &[ModelFile],
    config: &Config,
    project_dir: &Path,
    config_python: Option<&str>,
) -> Result<Vec<ModelFile>> {
    if python_files.is_empty() {
        return Ok(Vec::new());
    }

    // With PyO3, we don't need a separate Python interpreter — use embedded.
    // find_python_sdk is still needed to put the SDK on sys.path.
    #[cfg(feature = "python")]
    let python = {
        let _ = config_python; // not needed with embedded interpreter
        String::new()
    };
    #[cfg(not(feature = "python"))]
    let python = find_python(config_python)?;
    let python_sdk_path = find_python_sdk(project_dir)?;

    let max_rounds = 5;
    let mut python_models: Vec<ModelFile> = Vec::new();

    // Pre-compute decorator maps once (they don't change across rounds)
    let decorator_maps: Vec<HashMap<String, usize>> = python_files
        .iter()
        .map(|(_, _, content)| build_decorator_map(content))
        .collect();

    for _round in 0..max_rounds {
        let context_json = build_project_context(sql_models, &python_models, config);
        let mut new_models = Vec::new();

        for ((file_path, _decorator_lines, _file_content), decorator_map) in
            python_files.iter().zip(decorator_maps.iter())
        {
            let outputs = run_python_model(&python, file_path, &context_json, &python_sdk_path)?;

            for output in outputs {
                // Look up the decorator line for this specific model function
                let source_line = decorator_map.get(&output.name).copied().unwrap_or(1);

                // Parse the returned SQL through smelt-parser
                let parse = smelt_parser::parse(&output.sql);

                let refs = if let Some(file) = smelt_parser::File::cast(parse.syntax()) {
                    crate::discovery::extract_refs(&file)
                } else {
                    Vec::new()
                };

                // Extract metadata from generated SQL frontmatter
                let model_metadata =
                    extract_file_metadata(&output.sql)
                        .ok()
                        .and_then(|fm| match fm {
                            FileMetadata::Single { metadata, .. } => Some(metadata),
                            FileMetadata::Multi { models } => models
                                .into_iter()
                                .find(|section| {
                                    section.metadata.name.as_deref() == Some(&output.name)
                                })
                                .map(|section| Box::new(section.metadata)),
                            FileMetadata::Empty => None,
                        });

                // Convert parse errors, attributing them to the Python file
                let parse_errors: Vec<smelt_parser::ParseError> = parse
                    .errors
                    .iter()
                    .map(|e| smelt_parser::ParseError {
                        message: format!(
                            "{} (in SQL generated by {}:{})",
                            e.message,
                            file_path.display(),
                            source_line
                        ),
                        range: e.range,
                    })
                    .collect();

                new_models.push(ModelFile {
                    name: output.name.clone(),
                    path: file_path.clone(),
                    content: output.sql,
                    refs,
                    parse_errors,
                    metadata: model_metadata,
                    kind: ModelKind::Python {
                        source_line,
                        queries: output
                            .queries
                            .iter()
                            .map(|q| PythonModelQuery {
                                kind: q.kind.clone(),
                                tag: q.tag.clone(),
                                directory: q.directory.clone(),
                            })
                            .collect(),
                    },
                });
            }
        }

        // Check convergence: same set of models with same SQL
        if models_equal(&python_models, &new_models) {
            // Validate fixed-point: no model matches its own input queries
            validate_fixed_point(&new_models, config)?;
            return Ok(new_models);
        }

        python_models = new_models;
    }

    Err(anyhow!(
        "Python model discovery did not converge after {} rounds. \
         This likely indicates a circular dependency in Python model generation.",
        max_rounds
    ))
}

fn models_equal(a: &[ModelFile], b: &[ModelFile]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut a_sorted: Vec<(&str, &str)> = a
        .iter()
        .map(|m| (m.name.as_str(), m.content.as_str()))
        .collect();
    let mut b_sorted: Vec<(&str, &str)> = b
        .iter()
        .map(|m| (m.name.as_str(), m.content.as_str()))
        .collect();
    a_sorted.sort();
    b_sorted.sort();
    a_sorted == b_sorted
}

/// Validate that no Python-produced model matches its own input queries.
/// This prevents meta-level circular dependencies.
fn validate_fixed_point(models: &[ModelFile], config: &Config) -> Result<()> {
    for model in models {
        if let ModelKind::Python { queries, .. } = &model.kind {
            let model_tags =
                config.get_tags(&model.name, model.metadata.as_ref().map(|b| b.as_ref()));
            let model_directory = model
                .path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str());

            for query in queries {
                if query.kind == "find_models" {
                    if let Some(ref tag) = query.tag {
                        if model_tags.contains(tag) {
                            return Err(anyhow!(
                                "Python model '{}' calls find_models(tag=\"{}\") but the produced \
                                 model itself has that tag. This would create a circular dependency \
                                 at the meta level.",
                                model.name,
                                tag
                            ));
                        }
                    }
                    if let Some(ref dir) = query.directory {
                        if model_directory == Some(dir.as_str()) {
                            return Err(anyhow!(
                                "Python model '{}' calls find_models(directory=\"{}\") but the \
                                 produced model itself is in that directory. This would create a \
                                 circular dependency at the meta level.",
                                model.name,
                                dir
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// A query recorded during Python model execution.
#[derive(Debug, Clone)]
pub struct PythonModelQuery {
    pub kind: String,
    pub tag: Option<String>,
    pub directory: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(lines, vec![4, 11]);
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
        assert_eq!(lines, vec![6]);
    }

    #[test]
    fn test_python_model_end_to_end() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        // Create python SDK
        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();

        // Copy SDK files from the repo
        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");

        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        // Create model directory
        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        // Create a Python model
        std::fs::write(
            models_dir.join("dynamic_model.py"),
            r#"
from smelt import model

@model
def dynamic_model(project):
    return "SELECT 1 as id, 'hello' as greeting"
"#,
        )
        .unwrap();

        // Create a SQL model that refs the Python model
        std::fs::write(
            models_dir.join("downstream.sql"),
            "SELECT id FROM smelt.ref('dynamic_model')",
        )
        .unwrap();

        // Discover SQL models
        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let sql_models = discovery.discover_models().unwrap();
        assert_eq!(sql_models.len(), 1); // just downstream.sql

        // Discover Python files
        let python_files = discovery.discover_python_files().unwrap();
        assert_eq!(python_files.len(), 1);

        // Execute Python models
        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
        };

        let python_models =
            discover_python_models(&python_files, &sql_models, &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 1);
        assert_eq!(python_models[0].name, "dynamic_model");
        assert!(python_models[0].content.contains("SELECT 1"));

        // Verify the dependency graph works with mixed models
        let mut all_models = sql_models;
        all_models.extend(python_models);
        let graph = crate::graph::DependencyGraph::build(all_models, None).unwrap();
        graph.validate().unwrap();

        let order = graph.execution_order().unwrap();
        assert_eq!(order.len(), 2);
        // dynamic_model should come before downstream
        let dm_pos = order.iter().position(|n| n == "dynamic_model").unwrap();
        let ds_pos = order.iter().position(|n| n == "downstream").unwrap();
        assert!(dm_pos < ds_pos);
    }

    #[test]
    fn test_python_model_with_refs_in_generated_sql() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        // Set up SDK
        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");
        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        // Python model that generates SQL with smelt.ref()
        std::fs::write(
            models_dir.join("union_model.py"),
            r#"
from smelt import model

@model
def union_model(project):
    return "SELECT * FROM smelt.ref('table_a') UNION ALL SELECT * FROM smelt.ref('table_b')"
"#,
        )
        .unwrap();

        // SQL models that the Python model references
        std::fs::write(models_dir.join("table_a.sql"), "SELECT 1 as id").unwrap();
        std::fs::write(models_dir.join("table_b.sql"), "SELECT 2 as id").unwrap();

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let sql_models = discovery.discover_models().unwrap();
        let python_files = discovery.discover_python_files().unwrap();

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
        };

        let python_models =
            discover_python_models(&python_files, &sql_models, &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 1);
        assert_eq!(python_models[0].name, "union_model");
        // Verify refs were extracted from the generated SQL
        assert_eq!(python_models[0].refs.len(), 2);
        assert_eq!(python_models[0].refs[0].model_name, "table_a");
        assert_eq!(python_models[0].refs[1].model_name, "table_b");
    }

    // --- New tests for PR review fixes ---

    #[test]
    fn test_models_equal_order_independent() {
        // Fix #1: models_equal should not depend on order
        let model_a = ModelFile {
            name: "alpha".to_string(),
            path: PathBuf::from("a.py"),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
        };
        let model_b = ModelFile {
            name: "beta".to_string(),
            path: PathBuf::from("b.py"),
            content: "SELECT 2".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
        };

        let set1 = vec![model_a.clone(), model_b.clone()];
        let set2 = vec![model_b, model_a];
        assert!(models_equal(&set1, &set2));
    }

    #[test]
    fn test_models_equal_different_content() {
        let model_a = ModelFile {
            name: "same_name".to_string(),
            path: PathBuf::from("a.py"),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
        };
        let model_b = ModelFile {
            name: "same_name".to_string(),
            path: PathBuf::from("a.py"),
            content: "SELECT 2".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
        };

        assert!(!models_equal(&[model_a], &[model_b]));
    }

    #[test]
    fn test_build_decorator_map() {
        let content = r#"from smelt import model

@model
def first_model(project):
    return "SELECT 1"

@model
def second_model(project):
    return "SELECT 2"
"#;
        let map = build_decorator_map(content);
        assert_eq!(map.get("first_model"), Some(&3));
        assert_eq!(map.get("second_model"), Some(&7));
    }

    #[test]
    fn test_build_decorator_map_with_gaps() {
        // Blank lines between @model and def should be handled
        let content = r#"from smelt import model

@model

def my_model(project):
    return "SELECT 1"
"#;
        let map = build_decorator_map(content);
        assert_eq!(map.get("my_model"), Some(&3));
    }

    #[test]
    fn test_scan_decorator_with_parens() {
        let content = r#"from smelt import model

@model()
def my_model(project):
    return "SELECT 1"
"#;
        let lines = scan_for_model_decorators(content);
        assert_eq!(lines, vec![3]);

        let map = build_decorator_map(content);
        assert_eq!(map.get("my_model"), Some(&3));
    }

    #[test]
    fn test_multiple_models_one_file() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");
        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        let py_content = r#"from smelt import model

@model
def model_one(project):
    return "SELECT 1 as id"

@model
def model_two(project):
    return "SELECT 2 as id"
"#;
        std::fs::write(models_dir.join("multi.py"), py_content).unwrap();

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let python_files = discovery.discover_python_files().unwrap();
        assert_eq!(python_files.len(), 1);

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
        };

        let python_models =
            discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 2);
        let names: Vec<&str> = python_models.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"model_one"));
        assert!(names.contains(&"model_two"));

        // Verify source_line attribution (Fix #7)
        for m in &python_models {
            if let ModelKind::Python { source_line, .. } = &m.kind {
                match m.name.as_str() {
                    "model_one" => assert_eq!(*source_line, 3),
                    "model_two" => assert_eq!(*source_line, 7),
                    _ => panic!("unexpected model name"),
                }
            }
        }
    }

    #[test]
    fn test_find_models_convergence() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");
        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        // Create SQL models tagged as "event_source"
        std::fs::write(
            models_dir.join("page_views.sql"),
            "---\ntags:\n  - event_source\n---\nSELECT 1 as event_id",
        )
        .unwrap();
        std::fs::write(
            models_dir.join("clicks.sql"),
            "---\ntags:\n  - event_source\n---\nSELECT 2 as event_id",
        )
        .unwrap();

        // Python model that uses find_models
        std::fs::write(
            models_dir.join("combined.py"),
            r#"from smelt import model

@model
def combined(project):
    children = project.find_models(tag="event_source")
    if not children:
        return "SELECT 1 as event_id"
    refs = [f"SELECT * FROM smelt.ref('{m.name}')" for m in children]
    return " UNION ALL ".join(refs)
"#,
        )
        .unwrap();

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let sql_models = discovery.discover_models().unwrap();
        let python_files = discovery.discover_python_files().unwrap();

        let mut model_config = std::collections::HashMap::new();
        model_config.insert(
            "page_views".to_string(),
            crate::config::ModelConfig {
                materialization: None,
                incremental: None,
                tags: vec!["event_source".to_string()],
            },
        );
        model_config.insert(
            "clicks".to_string(),
            crate::config::ModelConfig {
                materialization: None,
                incremental: None,
                tags: vec!["event_source".to_string()],
            },
        );

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: model_config,
            python: None,
        };

        let python_models =
            discover_python_models(&python_files, &sql_models, &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 1);
        assert_eq!(python_models[0].name, "combined");
        // Should reference both event_source models
        assert!(python_models[0].content.contains("page_views"));
        assert!(python_models[0].content.contains("clicks"));
    }

    #[test]
    fn test_circular_meta_dependency() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");
        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        // Python model that queries tag "generated" but also produces a model with that tag
        std::fs::write(
            models_dir.join("circular.py"),
            r#"from smelt import model

@model
def circular_model(project):
    children = project.find_models(tag="generated")
    return "SELECT 1"
"#,
        )
        .unwrap();

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let python_files = discovery.discover_python_files().unwrap();

        let mut model_config = std::collections::HashMap::new();
        model_config.insert(
            "circular_model".to_string(),
            crate::config::ModelConfig {
                materialization: None,
                incremental: None,
                tags: vec!["generated".to_string()],
            },
        );

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: model_config,
            python: None,
        };

        let result = discover_python_models(&python_files, &[], &config, project_dir, None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("circular dependency"), "got: {}", err_msg);
    }

    #[test]
    fn test_bad_python_syntax() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");
        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        let bad_content = r#"from smelt import model

@model
def bad_model(project)
    return "SELECT 1"
"#;
        std::fs::write(models_dir.join("bad_syntax.py"), bad_content).unwrap();

        let python_files = vec![(
            models_dir.join("bad_syntax.py"),
            vec![3],
            bad_content.to_string(),
        )];

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
        };

        let result = discover_python_models(&python_files, &[], &config, project_dir, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_non_string_return() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");
        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        let py_content = r#"from smelt import model

@model
def bad_return(project):
    return 42
"#;
        std::fs::write(models_dir.join("bad_return.py"), py_content).unwrap();

        let python_files = vec![(
            models_dir.join("bad_return.py"),
            vec![3],
            py_content.to_string(),
        )];

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
        };

        let result = discover_python_models(&python_files, &[], &config, project_dir, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_sql_output() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");
        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        let py_content = r#"from smelt import model

@model
def bad_sql(project):
    return "NOT VALID SQL !!! SELECT FROM WHERE"
"#;
        std::fs::write(models_dir.join("bad_sql.py"), py_content).unwrap();

        let python_files = vec![(
            models_dir.join("bad_sql.py"),
            vec![3],
            py_content.to_string(),
        )];

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
        };

        let result =
            discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();
        // Model is produced but with parse errors
        assert_eq!(result.len(), 1);
        assert!(!result[0].parse_errors.is_empty());
    }

    #[test]
    fn test_empty_find_models() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");
        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        let py_content = r#"from smelt import model

@model
def no_matches(project):
    children = project.find_models(tag="nonexistent_tag")
    if not children:
        return "SELECT 1 as fallback"
    return "SELECT 2"
"#;
        std::fs::write(models_dir.join("no_matches.py"), py_content).unwrap();

        let python_files = vec![(
            models_dir.join("no_matches.py"),
            vec![3],
            py_content.to_string(),
        )];

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
        };

        let result =
            discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "no_matches");
        assert!(result[0].content.contains("fallback"));
    }

    #[test]
    fn test_python_model_name_collision() {
        // When a Python model produces a model with the same name as an existing SQL model,
        // the graph build silently overwrites. This test documents the current behavior.
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");
        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        // SQL model named "colliding"
        std::fs::write(models_dir.join("colliding.sql"), "SELECT 1 as from_sql").unwrap();

        // Python model that also produces "colliding"
        let py_content = r#"from smelt import model

@model
def colliding(project):
    return "SELECT 2 as from_python"
"#;
        std::fs::write(models_dir.join("gen_colliding.py"), py_content).unwrap();

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let sql_models = discovery.discover_models().unwrap();
        let python_files = discovery.discover_python_files().unwrap();

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
        };

        let python_models =
            discover_python_models(&python_files, &sql_models, &config, project_dir, None).unwrap();

        // Both exist but graph will have collision - one overwrites the other
        let mut all_models = sql_models;
        all_models.extend(python_models);
        let colliding_count = all_models.iter().filter(|m| m.name == "colliding").count();
        assert_eq!(
            colliding_count, 2,
            "both models should exist before graph build"
        );

        let graph = crate::graph::DependencyGraph::build(all_models, None).unwrap();
        // Graph silently keeps one (HashMap semantics) - documenting current behavior
        let order = graph.execution_order().unwrap();
        let colliding_in_order = order.iter().filter(|n| *n == "colliding").count();
        assert_eq!(colliding_in_order, 1, "graph deduplicates by name");
    }

    #[test]
    fn test_validate_fixed_point_detects_circular_tag() {
        let metadata = crate::metadata::ModelMetadata {
            tags: vec!["event_source".to_string()],
            ..Default::default()
        };

        let model = ModelFile {
            name: "combined_events".to_string(),
            path: PathBuf::from("models/combined_events.py"),
            content: "SELECT 1".to_string(),
            refs: Vec::new(),
            parse_errors: Vec::new(),
            metadata: Some(Box::new(metadata)),
            kind: ModelKind::Python {
                source_line: 1,
                queries: vec![PythonModelQuery {
                    kind: "find_models".to_string(),
                    tag: Some("event_source".to_string()),
                    directory: None,
                }],
            },
        };

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
        };

        let result = validate_fixed_point(&[model], &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("circular"));
    }

    #[test]
    fn test_validate_fixed_point_no_false_positive() {
        let metadata = crate::metadata::ModelMetadata {
            tags: vec!["output_model".to_string()],
            ..Default::default()
        };

        let model = ModelFile {
            name: "combined_events".to_string(),
            path: PathBuf::from("models/combined_events.py"),
            content: "SELECT 1".to_string(),
            refs: Vec::new(),
            parse_errors: Vec::new(),
            metadata: Some(Box::new(metadata)),
            kind: ModelKind::Python {
                source_line: 1,
                queries: vec![PythonModelQuery {
                    kind: "find_models".to_string(),
                    tag: Some("event_source".to_string()),
                    directory: None,
                }],
            },
        };

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
        };

        let result = validate_fixed_point(&[model], &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_non_model_python_files_skipped() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let models_dir = tmp.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        // A Python file without @model decorator
        std::fs::write(
            models_dir.join("helper.py"),
            "def some_utility(): return 42\n",
        )
        .unwrap();

        // A SQL model
        std::fs::write(models_dir.join("test.sql"), "SELECT 1").unwrap();

        let discovery = crate::discovery::ModelDiscovery::new(
            tmp.path().to_path_buf(),
            vec!["models".to_string()],
        );
        let python_files = discovery.discover_python_files().unwrap();
        assert!(python_files.is_empty());
    }
}
