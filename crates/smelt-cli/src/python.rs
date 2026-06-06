//! Python model discovery and execution.
//!
//! Python models are the "escape hatch" — deliberately low-level, for the ~5% of cases
//! where you need programmatic model generation (e.g., dynamically union all models with a tag).
//! Python models return SQL strings that get parsed by the existing smelt parser.

#[cfg(not(feature = "python"))]
use anyhow::Context;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(feature = "python"))]
use std::process::{Command, Stdio};

use crate::config::Config;
use crate::discovery::{ModelFile, ModelKind};
use crate::errors::CliError;
use crate::metadata::{extract_file_metadata, FileMetadata};
use smelt_core::python_utils::{self, ProjectContextData, ProjectModelInfo};
use smelt_core::ModelId;

/// Output from a single Python model function.
#[derive(Debug, Deserialize)]
struct PythonModelOutput {
    name: String,
    sql: String,
    #[serde(default)]
    queries: Vec<PythonModelQuery>,
}

// Shared helpers (find_python, find_python_sdk, scan_for_model_decorators,
// build_decorator_map, build_pythonpath) are in smelt_core::python_utils.

/// Execute a Python model file and return the generated SQL models (subprocess path).
#[cfg(not(feature = "python"))]
fn run_python_model(
    python: &str,
    file_path: &Path,
    project_context_json: &str,
    python_sdk_path: &Path,
) -> Result<Vec<PythonModelOutput>> {
    let pythonpath = python_utils::build_pythonpath(python_sdk_path, file_path);
    // Pass context via stdin to avoid OS argument size limits (E2BIG).
    let mut child = Command::new(python)
        .arg("-m")
        .arg("smelt.runner")
        .arg(file_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONPATH", pythonpath)
        .spawn()
        .with_context(|| format!("Failed to execute Python model: {}", file_path.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(project_context_json.as_bytes())
            .with_context(|| {
                format!(
                    "Failed to write context to Python model: {}",
                    file_path.display()
                )
            })?;
    }
    let output = child
        .wait_with_output()
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

/// Discover and execute Python models.
///
/// Uses iterative discovery with fixed-point validation:
/// 1. Run all Python models with current project context
/// 2. If new models produced, rebuild context and re-run
/// 3. Stop when output stabilizes (or max rounds reached)
pub fn discover_python_models(
    python_files: &[(PathBuf, Vec<u32>, String)], // (path, decorator_lines, content)
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
    let python = python_utils::find_python(config_python)
        .ok_or::<anyhow::Error>(CliError::PythonNotFound.into())?;
    let python_sdk_path = python_utils::find_python_sdk(project_dir).ok_or_else(|| {
        anyhow!(
            "smelt Python SDK not found. Expected a 'python/smelt/' directory in or above the project root.\n\
             Hint: Set SMELT_PYTHON_SDK to the directory containing the smelt Python package."
        )
    })?;

    let max_rounds = 5;
    let mut python_models: Vec<ModelFile> = Vec::new();

    // Pre-compute decorator maps once (they don't change across rounds)
    let decorator_maps: Vec<HashMap<String, u32>> = python_files
        .iter()
        .map(|(_, _, content)| python_utils::build_decorator_map(content))
        .collect();

    for _round in 0..max_rounds {
        let context_json = build_project_context(sql_models, &python_models, config);
        let mut new_models = Vec::new();

        for ((file_path, _decorator_lines, _file_content), decorator_map) in
            python_files.iter().zip(decorator_maps.iter())
        {
            let outputs = run_python_model(&python, file_path, &context_json, &python_sdk_path)?;

            for output in outputs {
                // Look up the decorator line for this specific model function (convert 0-indexed to 1-indexed)
                let source_line = decorator_map
                    .get(&output.name)
                    .map(|&line| line as usize + 1)
                    .unwrap_or(1);

                // Parse the returned SQL through smelt-parser
                let parse = smelt_parser::parse(&output.sql);

                let refs = if let Some(file) = smelt_parser::File::cast(parse.syntax()) {
                    crate::discovery::extract_refs(&file)
                } else {
                    Vec::new()
                };

                // Extract metadata from generated SQL frontmatter, checking for
                // name mismatches between the frontmatter `name:` field and the
                // Python function name (BUG-038).
                let mut name_mismatch_error: Option<smelt_parser::ParseError> = None;
                let model_metadata = {
                    let fm_opt = match extract_file_metadata(&output.sql) {
                        Ok(fm) => Some(fm),
                        Err(e) => {
                            tracing::warn!("Python model {}: {}", output.name, e);
                            None
                        }
                    };
                    match fm_opt {
                        Some(FileMetadata::Single { metadata, .. }) => {
                            // If the frontmatter declares a `name:` that differs from
                            // the function name, emit PythonModelNameMismatch and drop
                            // the frontmatter so defaults apply.
                            if let Some(ref fm_name) = metadata.name {
                                if fm_name != &output.name {
                                    name_mismatch_error = Some(smelt_parser::ParseError {
                                        message: format!(
                                            "PythonModelNameMismatch: frontmatter declares \
                                             name '{}' but function name is '{}'; remove \
                                             the name field or set it to '{}'",
                                            fm_name, output.name, output.name
                                        ),
                                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                                    });
                                    None
                                } else {
                                    Some(metadata)
                                }
                            } else {
                                Some(metadata)
                            }
                        }
                        Some(FileMetadata::Multi { models }) => {
                            // In multi-section output, each section is identified by
                            // its `name:` field.  Collect the section names first so
                            // we can report them on a mismatch, then find the match.
                            let section_names: Vec<String> = models
                                .iter()
                                .filter_map(|s| s.metadata.name.clone())
                                .collect();
                            let matched = models
                                .into_iter()
                                .find(|s| s.metadata.name.as_deref() == Some(&output.name));
                            if matched.is_none() {
                                // No section matched the function name — mismatch.
                                let declared = if section_names.is_empty() {
                                    "(none)".to_string()
                                } else {
                                    section_names
                                        .iter()
                                        .map(|n| format!("'{n}'"))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                };
                                name_mismatch_error = Some(smelt_parser::ParseError {
                                    message: format!(
                                        "PythonModelNameMismatch: frontmatter section \
                                         name(s) {declared} do not match function name \
                                         '{}'; the frontmatter name must match the \
                                         function name",
                                        output.name
                                    ),
                                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                                });
                            }
                            matched.map(|section| Box::new(section.metadata))
                        }
                        Some(FileMetadata::Empty) | None => None,
                        // Generator files produce models via meta-language evaluation;
                        // Python model output is not a generator file.
                        Some(FileMetadata::Generator { .. }) => None,
                    }
                };

                // Convert parse errors, attributing them to the Python file.
                // Also include the name-mismatch sentinel if one was detected.
                let mut parse_errors: Vec<smelt_parser::ParseError> = parse
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
                if let Some(mismatch_err) = name_mismatch_error {
                    parse_errors.push(mismatch_err);
                }

                let model_id = ModelId::from_path(file_path.clone());
                new_models.push(ModelFile {
                    name: output.name.clone(),
                    path: file_path.clone(),
                    content: output.sql,
                    refs,
                    parse_errors,
                    metadata: model_metadata,
                    kind: ModelKind::Python {
                        source_line,
                        queries: output.queries.clone(),
                    },
                    model_id,
                    // Python model address is the function name (a single-segment address).
                    // This enables `resolve_address_map` to detect Python-vs-SQL collisions.
                    address_segments: vec![output.name.clone()],
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

// `PythonModelQuery` is shared with `smelt-core` so the type that flows
// through `ModelFile::kind` is the same one the PyO3 runner emits.
pub use smelt_core::PythonModelQuery;

#[cfg(test)]
mod tests {
    use super::*;

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
            "SELECT id FROM smelt.models.dynamic_model",
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
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
        };

        let python_models =
            discover_python_models(&python_files, &sql_models, &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 1);
        assert_eq!(python_models[0].name, "dynamic_model");
        assert!(python_models[0].content.contains("SELECT 1"));

        // Verify the dependency graph works with mixed models
        let mut all_models = sql_models;
        all_models.extend(python_models);
        let graph =
            crate::logical_graph::LogicalGraph::build(all_models, None, &[], &config, "dev")
                .unwrap();
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

        // Python model that generates SQL with smelt.models.* path refs
        std::fs::write(
            models_dir.join("union_model.py"),
            r#"
from smelt import model

@model
def union_model(project):
    return "SELECT * FROM smelt.models.table_a UNION ALL SELECT * FROM smelt.models.table_b"
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
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
        };

        let python_models =
            discover_python_models(&python_files, &sql_models, &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 1);
        assert_eq!(python_models[0].name, "union_model");
        // Verify refs were extracted from the generated SQL
        assert_eq!(python_models[0].refs.len(), 2);
        assert_eq!(python_models[0].refs[0].smelt_ref.leaf_name(), "table_a");
        assert_eq!(python_models[0].refs[1].smelt_ref.leaf_name(), "table_b");
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
            model_id: ModelId::from_path(PathBuf::from("test.sql")),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
            address_segments: Vec::new(),
        };
        let model_b = ModelFile {
            name: "beta".to_string(),
            path: PathBuf::from("b.py"),
            content: "SELECT 2".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(PathBuf::from("test.sql")),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
            address_segments: Vec::new(),
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
            model_id: ModelId::from_path(PathBuf::from("test.sql")),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
            address_segments: Vec::new(),
        };
        let model_b = ModelFile {
            name: "same_name".to_string(),
            path: PathBuf::from("a.py"),
            content: "SELECT 2".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(PathBuf::from("test.sql")),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
            address_segments: Vec::new(),
        };

        assert!(!models_equal(&[model_a], &[model_b]));
    }

    // build_decorator_map and scan_for_model_decorators tests are in smelt_core::python_utils

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
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
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
    fn test_model_called_form_recognized() {
        // Regression for BUG-039: the spec (python_models.md §Surface — `@model`
        // decorator) states "Both `@model` and `@model()` (called form) are
        // recognized." The Rust/LSP scanner already accepts `@model()`, but the
        // Python SDK `model` decorator must also accept the called form rather
        // than raising `TypeError: model() missing 1 required positional argument`.
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

@model()
def called_form(project):
    return "SELECT 1 as id"
"#;
        std::fs::write(models_dir.join("called.py"), py_content).unwrap();

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let python_files = discovery.discover_python_files().unwrap();
        assert_eq!(python_files.len(), 1);

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
        };

        let python_models =
            discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 1);
        assert_eq!(python_models[0].name, "called_form");
        assert!(python_models[0].content.contains("SELECT 1"));
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
    refs = [f"SELECT * FROM smelt.models.{m.name}" for m in children]
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
                timeseries: None,
                incremental: None,
                tags: vec!["event_source".to_string()],
                target: None,
            },
        );
        model_config.insert(
            "clicks".to_string(),
            crate::config::ModelConfig {
                materialization: None,
                timeseries: None,
                incremental: None,
                tags: vec!["event_source".to_string()],
                target: None,
            },
        );

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: model_config,
            python: None,
            target: None,
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
                timeseries: None,
                incremental: None,
                tags: vec!["generated".to_string()],
                target: None,
            },
        );

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: model_config,
            python: None,
            target: None,
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
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
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
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
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
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
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
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
        };

        let result =
            discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "no_matches");
        assert!(result[0].content.contains("fallback"));
    }

    #[test]
    fn test_python_model_name_collision() {
        // BUG-040: a Python @model whose name matches a SQL model's canonical address
        // must be rejected with a DuplicateAddress error (not silently deduped).
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
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
        };

        let python_models =
            discover_python_models(&python_files, &sql_models, &config, project_dir, None).unwrap();

        // Both exist in the combined Vec before graph build.
        let mut all_models = sql_models;
        all_models.extend(python_models);
        let colliding_count = all_models.iter().filter(|m| m.name == "colliding").count();
        assert_eq!(
            colliding_count, 2,
            "both models should exist before graph build"
        );

        // LogicalGraph::build must now reject the collision instead of silently deduping.
        let result =
            crate::logical_graph::LogicalGraph::build(all_models, None, &[], &config, "dev");
        assert!(
            result.is_err(),
            "expected DuplicateAddress error for Python-vs-SQL collision, got Ok"
        );
        let err_msg = result.err().expect("expected Err").to_string();
        assert!(
            err_msg.contains("DuplicateAddress") || err_msg.contains("colliding"),
            "error message should reference the collision: {err_msg}"
        );
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
            model_id: ModelId::from_path(PathBuf::from("models/combined_events.py")),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
            address_segments: Vec::new(),
        };

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
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
            model_id: ModelId::from_path(PathBuf::from("models/combined_events.py")),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
            address_segments: Vec::new(),
        };

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
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

    /// BUG-038: a Python `@model` whose returned SQL starts with
    /// `--- name: X ---` frontmatter where X ≠ function name must produce a
    /// `PythonModelNameMismatch` error rather than silently dropping the
    /// frontmatter (materialization etc.) and registering under the function name
    /// with view defaults.
    ///
    /// Before the fix the model would be produced with `metadata = None`
    /// (view default) and `parse_errors.is_empty()`.
    /// After the fix the model is produced (not a hard error) but
    /// `parse_errors` contains exactly one entry whose message starts with
    /// `"PythonModelNameMismatch"`.
    #[test]
    fn test_python_model_frontmatter_name_mismatch_emits_diagnostic() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        // Set up SDK
        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let repo_sdk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
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

        // Python model that returns SQL with a frontmatter `name:` that
        // differs from the function name.  Before BUG-038 fix this silently
        // dropped the `materialization: table` and registered the model as a
        // view with no diagnostic.
        let py_content = r#"from smelt import model

@model
def my_func(project):
    return """--- name: other_name ---
materialization: table
---
SELECT 1 AS id
"""
"#;
        std::fs::write(models_dir.join("mismatch.py"), py_content).unwrap();

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let python_files = discovery.discover_python_files().unwrap();
        assert_eq!(python_files.len(), 1);

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
        };

        // discover_python_models must succeed (not return Err) — the mismatch
        // is a per-model diagnostic, not a hard stop.
        let python_models =
            discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 1, "model must still be produced");
        let model = &python_models[0];

        // Model name is always the function name.
        assert_eq!(model.name, "my_func");

        // A PythonModelNameMismatch diagnostic must be present.
        let mismatch_errs: Vec<_> = model
            .parse_errors
            .iter()
            .filter(|e| e.message.starts_with("PythonModelNameMismatch"))
            .collect();
        assert_eq!(
            mismatch_errs.len(),
            1,
            "expected exactly one PythonModelNameMismatch parse error; got: {:#?}",
            model.parse_errors
        );
        // The message must mention both the frontmatter name and the function name.
        let msg = &mismatch_errs[0].message;
        assert!(
            msg.contains("other_name"),
            "error message must mention frontmatter name 'other_name'; got: {msg}"
        );
        assert!(
            msg.contains("my_func"),
            "error message must mention function name 'my_func'; got: {msg}"
        );

        // The frontmatter metadata must be dropped (mismatch → no metadata
        // applied; materialisation falls back to project default).
        assert!(
            model.metadata.is_none(),
            "metadata must be None when there is a name mismatch; got: {:#?}",
            model.metadata
        );
    }
}
