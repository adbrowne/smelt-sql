//! Python model discovery and execution.
//!
//! Python models are the "escape hatch" — deliberately low-level, for the ~5% of cases
//! where you need programmatic model generation (e.g., dynamically union all models with a tag).
//! Python models return SQL strings that get parsed by the existing smelt parser.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;
use crate::discovery::{ModelFile, ModelKind};
use crate::errors::CliError;

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
/// Resolution order: SMELT_PYTHON_SDK env var → project_dir/python/ → bundled with binary
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

/// Execute a Python model file and return the generated SQL models.
fn run_python_model(
    python: &str,
    file_path: &Path,
    project_context_json: &str,
    python_sdk_path: &Path,
) -> Result<Vec<PythonModelOutput>> {
    let output = Command::new(python)
        .arg("-m")
        .arg("smelt.runner")
        .arg(file_path)
        .arg(project_context_json)
        .env("PYTHONPATH", python_sdk_path)
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

/// Discover and execute Python models.
///
/// Uses iterative discovery with fixed-point validation:
/// 1. Run all Python models with current project context
/// 2. If new models produced, rebuild context and re-run
/// 3. Stop when output stabilizes (or max rounds reached)
pub fn discover_python_models(
    python_files: &[(PathBuf, Vec<usize>)], // (path, decorator_lines)
    sql_models: &[ModelFile],
    config: &Config,
    project_dir: &Path,
    config_python: Option<&str>,
) -> Result<Vec<ModelFile>> {
    if python_files.is_empty() {
        return Ok(Vec::new());
    }

    let python = find_python(config_python)?;
    let python_sdk_path = find_python_sdk(project_dir)?;

    let max_rounds = 5;
    let mut python_models: Vec<ModelFile> = Vec::new();

    for _round in 0..max_rounds {
        let context_json = build_project_context(sql_models, &python_models, config);
        let mut new_models = Vec::new();

        for (file_path, decorator_lines) in python_files {
            let outputs = run_python_model(&python, file_path, &context_json, &python_sdk_path)?;

            for output in outputs {
                // Find the decorator line for this model (use first if only one)
                let source_line = decorator_lines.first().copied().unwrap_or(1);

                // Parse the returned SQL through smelt-parser
                let parse = smelt_parser::parse(&output.sql);

                let refs = if let Some(file) = smelt_parser::File::cast(parse.syntax()) {
                    crate::discovery::extract_refs(&file)
                } else {
                    Vec::new()
                };

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
                    metadata: None,
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
    for (ma, mb) in a.iter().zip(b.iter()) {
        if ma.name != mb.name || ma.content != mb.content {
            return false;
        }
    }
    true
}

/// Validate that no Python-produced model matches its own input queries.
/// This prevents meta-level circular dependencies.
fn validate_fixed_point(models: &[ModelFile], config: &Config) -> Result<()> {
    for model in models {
        if let ModelKind::Python { queries, .. } = &model.kind {
            let model_tags =
                config.get_tags(&model.name, model.metadata.as_ref().map(|b| b.as_ref()));

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
