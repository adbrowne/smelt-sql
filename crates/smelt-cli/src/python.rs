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
    project_dir: &Path,
) -> String {
    let mut models = Vec::new();

    for model in sql_models.iter().chain(python_models.iter()) {
        let tags = config.get_tags(&model.name, model.metadata.as_ref().map(|b| b.as_ref()));

        // Full workspace-relative path, forward-slash normalised (D-25).
        let path = model.path.strip_prefix(project_dir).ok().map(|rel| {
            rel.to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        });

        // directory = final component of the parent directory, derived from path (D-25).
        let directory = path.as_deref().and_then(|p| {
            let slash = p.rfind('/')?;
            p[..slash]
                .split('/')
                .next_back()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });

        models.push(ProjectModelInfo {
            name: model.name.clone(),
            tags,
            directory,
            path,
        });
    }

    let context = ProjectContextData { models };
    serde_json::to_string(&context).expect("Failed to serialize project context")
}

/// Normalize Python model SQL output to plain single-model frontmatter (D-22).
///
/// Python output uses `--- name: X ---` section-delimiter format in some cases,
/// but Python output is always single-model. Rewrite the first line when it is
/// a section delimiter so downstream code sees standard `---` frontmatter.
fn normalize_python_sql(sql: &str) -> std::borrow::Cow<'_, str> {
    let first_line_end = sql.find('\n').unwrap_or(sql.len());
    let first_line = sql[..first_line_end].trim();
    if first_line.starts_with("--- name:") && first_line.ends_with("---") && first_line != "---" {
        let after_prefix = &first_line[9..]; // skip "--- name:"
        let name_part = &after_prefix[..after_prefix.len() - 3]; // remove " ---"
        let name = name_part.trim();
        let rest = &sql[first_line_end..]; // includes leading '\n' or is empty
        std::borrow::Cow::Owned(format!("---\nname: {name}{rest}"))
    } else {
        std::borrow::Cow::Borrowed(sql)
    }
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
        let context_json = build_project_context(sql_models, &python_models, config, project_dir);
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

                // D-22: Python output is always single-model. Normalize
                // `--- name: X ---` section-delimiter format to plain frontmatter
                // so downstream code sees a consistent `---` block.
                let normalized_sql = normalize_python_sql(&output.sql);

                // Strip frontmatter before SQL parsing to avoid spurious parse
                // errors from YAML keys being interpreted as SQL identifiers.
                let clean_sql = smelt_parser::strip_frontmatter(&normalized_sql);
                let parse = smelt_parser::parse(&clean_sql);

                let refs = if let Some(file) = smelt_parser::File::cast(parse.syntax()) {
                    crate::discovery::extract_refs(&file)
                } else {
                    Vec::new()
                };

                // Extract metadata from the normalized SQL frontmatter, checking
                // for name mismatches between the frontmatter `name:` field and
                // the Python function name (D-27).
                let mut name_mismatch_error: Option<smelt_parser::ParseError> = None;
                let model_metadata = {
                    let fm_opt = match extract_file_metadata(&normalized_sql) {
                        Ok(fm) => Some(fm),
                        Err(e) => {
                            tracing::warn!("Python model {}: {}", output.name, e);
                            None
                        }
                    };
                    match fm_opt {
                        Some(FileMetadata::Single { metadata, .. }) => {
                            if let Some(ref fm_name) = metadata.name {
                                if fm_name != &output.name {
                                    // D-27: emit mismatch error AND retain other
                                    // frontmatter keys (only `name:` is stripped).
                                    name_mismatch_error = Some(smelt_parser::ParseError {
                                        message: format!(
                                            "PythonModelNameMismatch: frontmatter declares \
                                             name '{}' but function name is '{}'; remove \
                                             the name field or set it to '{}'",
                                            fm_name, output.name, output.name
                                        ),
                                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                                    });
                                    let mut retained = metadata;
                                    retained.name = None;
                                    Some(retained)
                                } else {
                                    Some(metadata)
                                }
                            } else {
                                Some(metadata)
                            }
                        }
                        // D-22: Multi is unreachable after normalization.
                        Some(FileMetadata::Multi { .. }) => None,
                        Some(FileMetadata::Empty) | None => None,
                        // Generator files are not valid Python model output.
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
                // Path-derived address per D-26: py/archive.py::users → smelt.archive.users.
                // When the file stem equals the function name (e.g. combined_events.py::combined_events),
                // the stem already IS the leaf — don't push it twice. When stem ≠ function (multi-function
                // files like archive.py::users), the stem is a namespace prefix and the function is the leaf.
                let address_segments = {
                    let mut segs = smelt_core::discovery::ModelDiscovery::compute_address_segments(
                        file_path,
                        project_dir,
                        &config.paths,
                    );
                    if segs.last().map(|s| s.as_str()) != Some(output.name.as_str()) {
                        segs.push(output.name.clone());
                    }
                    segs
                };
                new_models.push(ModelFile {
                    name: output.name.clone(),
                    path: file_path.clone(),
                    content: normalized_sql.into_owned(),
                    refs,
                    parse_errors,
                    metadata: model_metadata,
                    kind: ModelKind::Python {
                        source_line,
                        queries: output.queries.clone(),
                    },
                    model_id,
                    address_segments,
                });
            }
        }

        // Check convergence: same set of models with same SQL.
        // A stable fixed-point is accepted regardless of self-referential
        // queries — circularity is non-convergence, not self-tag/self-dir (D-23).
        if models_equal(&python_models, &new_models) {
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

        // Create a Python model in gen.py (stem "gen") with function "dynamic_model".
        // With paths: ["models"], D-26 address = smelt.gen.dynamic_model.
        std::fs::write(
            models_dir.join("gen.py"),
            r#"
from smelt import model

@model
def dynamic_model(project):
    return "SELECT 1 as id, 'hello' as greeting"
"#,
        )
        .unwrap();

        // Create a SQL model that refs the Python model at its canonical D-26 address.
        std::fs::write(
            models_dir.join("downstream.sql"),
            "SELECT id FROM smelt.gen.dynamic_model",
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
        let graph = smelt_core::graph::DependencyGraph::build(all_models, None).unwrap();
        graph.validate().unwrap();

        let order = graph.execution_order().unwrap();
        assert_eq!(order.len(), 2);
        // gen.dynamic_model (canonical path) should come before downstream
        let dm_pos = order.iter().position(|n| n == "gen.dynamic_model").unwrap();
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
        let model_a = ModelFile {
            name: "alpha".to_string(),
            path: PathBuf::from("a.py"),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(PathBuf::from("test.sql")),
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

    // A generator that emits a model carrying a tag it also queries converges
    // (the output stabilises after round 2) and must NOT be rejected as circular.
    // D-23: circularity = non-convergence, not self-tag/self-dir.
    #[test]
    fn self_referential_convergent_generation_is_legal() {
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

        // A generator that queries a tag it also carries.  It ignores the
        // children and always returns "SELECT 1", so the output is stable after
        // the first round — this is the supported self-referential-generation
        // pattern, not a cycle.
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

        // Convergent: the model always returns "SELECT 1" regardless of what
        // find_models returns, so discovery stabilises after round 2.
        let result = discover_python_models(&python_files, &[], &config, project_dir, None);
        assert!(
            result.is_ok(),
            "convergent self-referential generator should be legal; got: {:?}",
            result.err()
        );
    }

    // A model whose output keeps changing across all rounds triggers the
    // non-convergence error (the only valid circularity signal per D-23).
    #[test]
    fn non_convergent_set_errors() {
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

        // A counter file so the model returns different SQL on every call,
        // preventing convergence across all 5 rounds.
        let counter_file = tmp.path().join("counter.txt");
        std::fs::write(&counter_file, "0").unwrap();
        let counter_path = counter_file.display().to_string();

        let py_content = format!(
            r#"from smelt import model
import os

@model
def unstable(project):
    counter_file = r"{counter_path}"
    n = int(open(counter_file).read().strip())
    n += 1
    open(counter_file, "w").write(str(n))
    return f"SELECT {{n}}"
"#
        );
        std::fs::write(models_dir.join("unstable.py"), &py_content).unwrap();

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
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

        let result = discover_python_models(&python_files, &[], &config, project_dir, None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("converge") || err_msg.contains("circular"),
            "expected non-convergence error, got: {err_msg}"
        );
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

        // D-26: Python model in gen_colliding.py::colliding → path-derived segments.
        assert_eq!(python_models.len(), 1);
        assert_eq!(
            python_models[0].address_segments,
            vec!["gen_colliding", "colliding"],
            "Python model address should be [file_stem, func_name] = ['gen_colliding', 'colliding']"
        );

        // Both exist in the combined Vec before graph build (they have DIFFERENT
        // addresses: SQL = ["colliding"], Python = ["gen_colliding", "colliding"]).
        let mut all_models = sql_models;
        all_models.extend(python_models);
        let colliding_count = all_models.iter().filter(|m| m.name == "colliding").count();
        assert_eq!(
            colliding_count, 2,
            "both models should exist before graph build"
        );

        // DependencyGraph::build silently deduplicates (last wins) when there's a canonical-path
        // collision between Python and SQL models. With path-derived Python addresses, these two
        // no longer share a canonical path, so both survive in the graph.
        let result = smelt_core::graph::DependencyGraph::build(all_models, None);
        assert!(
            result.is_ok(),
            "DependencyGraph::build should succeed: {:?}",
            result.err()
        );
        // True Python↔SQL address collisions (same path-derived address) are detected by
        // smelt_core::resolver::resolve_address_map (Salsa layer); see
        // python_sql_address_collision_is_duplicate_address for the unit test.
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

    /// D-27: Python name mismatch must block the build AND retain other frontmatter keys.
    /// The `name:` field is the only thing flagged; `materialization:`, `tags:` etc. are kept.
    ///
    /// BUG-038 regression: before the fix the model was produced with `metadata = None`
    /// (view default) and `parse_errors.is_empty()`.
    #[test]
    fn python_name_mismatch_blocks_and_retains_other_keys() {
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

        // Python model that returns SQL with a frontmatter `name:` that differs
        // from the function name. Uses `--- name: X ---` section-delimiter format
        // (D-22: must be treated as single-model, not multi-model sections).
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

        // D-27: other frontmatter keys (materialization, tags) must be RETAINED on mismatch.
        // Only the `name:` key is flagged; the rest is applied to the model.
        let meta = model
            .metadata
            .as_ref()
            .expect("metadata must be retained (not None) on name mismatch (D-27)");
        assert_eq!(
            meta.materialization,
            Some(crate::config::Materialization::Table),
            "materialization must be retained from frontmatter despite name mismatch"
        );
        assert!(
            meta.name.is_none(),
            "name: field must be stripped from retained metadata; got: {:?}",
            meta.name
        );
    }

    /// D-22: Python output with plain `---` frontmatter (no `name:` key) is always
    /// treated as single-model. Identity comes from the function name.
    #[test]
    fn python_plain_frontmatter_single_model() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

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

        let py_content = r#"from smelt import model

@model
def my_model(project):
    return """---
materialization: table
---
SELECT 1 AS id
"""
"#;
        std::fs::write(models_dir.join("my_model.py"), py_content).unwrap();

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

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let python_files = discovery.discover_python_files().unwrap();
        let python_models =
            discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 1);
        let model = &python_models[0];
        assert_eq!(model.name, "my_model");
        assert!(
            model.parse_errors.is_empty(),
            "plain frontmatter must produce no errors; got: {:#?}",
            model.parse_errors
        );
        let meta = model
            .metadata
            .as_ref()
            .expect("metadata must be populated from plain frontmatter");
        assert_eq!(
            meta.materialization,
            Some(crate::config::Materialization::Table)
        );
    }

    /// D-22: `--- name: X ---` (section-delimiter format) in Python output must NOT
    /// create a multi-model section — Python output is always single-model.
    /// When X matches the function name, no error is produced and metadata is retained.
    #[test]
    fn python_multimodel_delimiter_not_a_section() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

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

        // Output uses `--- name: X ---` section-delimiter format where X matches
        // the function name. D-22: treated as single-model, not a section list.
        let py_content = r#"from smelt import model

@model
def matching_func(project):
    return """--- name: matching_func ---
materialization: table
---
SELECT 1 AS id
"""
"#;
        std::fs::write(models_dir.join("matching_func.py"), py_content).unwrap();

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

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let python_files = discovery.discover_python_files().unwrap();
        let python_models =
            discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();

        // D-22: exactly 1 model, not multiple from multi-model parsing.
        assert_eq!(
            python_models.len(),
            1,
            "section-delimiter format must produce exactly 1 model (D-22), not multiple"
        );
        let model = &python_models[0];
        assert_eq!(model.name, "matching_func");
        // D-22: when name matches, no PythonModelNameMismatch error.
        assert!(
            model.parse_errors.is_empty(),
            "matching name must produce no errors; got: {:#?}",
            model.parse_errors
        );
        // D-22: metadata from the frontmatter body is retained.
        let meta = model
            .metadata
            .as_ref()
            .expect("metadata must be populated from section-delimiter format (D-22)");
        assert_eq!(
            meta.materialization,
            Some(crate::config::Materialization::Table),
            "materialization must be retained"
        );
    }

    // D-26: discover_python_models must produce path-derived address_segments.
    // Integration test — uses actual Python model execution.
    #[test]
    fn python_discover_address_segments_are_path_derived() {
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

        // File `models/gen.py` containing function `output` — stem ≠ function name.
        // With paths: ["models"], expected address = ["gen", "output"] = smelt.gen.output.
        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(
            models_dir.join("gen.py"),
            r#"from smelt import model

@model
def output(project):
    return "SELECT 1"
"#,
        )
        .unwrap();

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

        let discovery = crate::discovery::ModelDiscovery::new(
            project_dir.to_path_buf(),
            vec!["models".to_string()],
        );
        let python_files = discovery.discover_python_files().unwrap();

        let models = discover_python_models(&python_files, &[], &config, project_dir, None)
            .expect("discover_python_models failed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "output");
        assert_eq!(
            models[0].address_segments,
            vec!["gen", "output"],
            "Python model address should be path-derived: file stem 'gen' + function name 'output'"
        );
    }

    // D-26: Python address = directory-prefix (file stem after paths: strip) + function name.
    // Pure unit test — no Python interpreter needed.
    #[test]
    fn python_address_is_path_derived() {
        use smelt_core::discovery::ModelDiscovery;
        use std::path::PathBuf;

        let root = PathBuf::from("/project");
        let paths = vec!["py".to_string()];

        // archive.py in py/ → compute_address_segments gives ["archive"] (stem after stripping py/)
        // + function name "users" → ["archive", "users"]
        let file = root.join("py").join("archive.py");
        let mut segs = ModelDiscovery::compute_address_segments(&file, &root, &paths);
        segs.push("users".to_string());
        assert_eq!(
            segs,
            vec!["archive", "users"],
            "py/archive.py::users should address as smelt.archive.users"
        );

        // root-level file in the paths dir: py/util.py + function "helper" → ["util", "helper"]
        let file2 = root.join("py").join("util.py");
        let mut segs2 = ModelDiscovery::compute_address_segments(&file2, &root, &paths);
        segs2.push("helper".to_string());
        assert_eq!(segs2, vec!["util", "helper"]);

        // subdirectory case: py/staging/stg.py + function "events" → ["staging", "stg", "events"]
        let file3 = root.join("py").join("staging").join("stg.py");
        let mut segs3 = ModelDiscovery::compute_address_segments(&file3, &root, &paths);
        segs3.push("events".to_string());
        assert_eq!(segs3, vec!["staging", "stg", "events"]);
    }

    // D-26: after the fix, a Python model whose path-derived address equals a SQL model's
    // address is a DuplicateAddress collision detectable via resolve_address_map.
    #[test]
    fn python_sql_address_collision_is_duplicate_address() {
        use smelt_core::resolver::resolve_address_map;

        // SQL model with address ["archive", "users"]
        let sql_model = ModelFile {
            name: "users".to_string(),
            path: PathBuf::from("/project/py/archive.sql"),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(PathBuf::from("/project/py/archive.sql")),
            address_segments: vec!["archive".to_string(), "users".to_string()],
        };

        // Python model with the same path-derived address ["archive", "users"]
        let python_model = ModelFile {
            name: "users".to_string(),
            path: PathBuf::from("/project/py/archive.py"),
            content: "SELECT 2".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Python {
                source_line: 1,
                queries: vec![],
            },
            model_id: ModelId::from_path(PathBuf::from("/project/py/archive.py")),
            address_segments: vec!["archive".to_string(), "users".to_string()],
        };

        let all_models = vec![sql_model, python_model];
        let (_map, collisions) = resolve_address_map(&all_models, &[], &[]);
        assert_eq!(
            collisions.len(),
            1,
            "Python model with same path-derived address as SQL model should produce DuplicateAddress collision"
        );
        assert_eq!(collisions[0].address, vec!["archive", "users"]);
    }

    // D-25: build_project_context exposes full workspace-relative path; directory is derived
    // from path (never disagrees).
    #[test]
    fn project_context_exposes_full_path() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        // Model at models/staging/stg_events.sql
        let model_path = project_dir
            .join("models")
            .join("staging")
            .join("stg_events.sql");
        let sql_model = ModelFile {
            name: "stg_events".to_string(),
            path: model_path,
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(
                project_dir
                    .join("models")
                    .join("staging")
                    .join("stg_events.sql"),
            ),
            address_segments: vec!["staging".to_string(), "stg_events".to_string()],
        };
        // Root-level model at models/flat.sql
        let flat_path = project_dir.join("models").join("flat.sql");
        let flat_model = ModelFile {
            name: "flat".to_string(),
            path: flat_path,
            content: "SELECT 2".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(project_dir.join("models").join("flat.sql")),
            address_segments: vec!["flat".to_string()],
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

        let context_json =
            build_project_context(&[sql_model, flat_model], &[], &config, project_dir);
        let context: smelt_core::python_utils::ProjectContextData =
            serde_json::from_str(&context_json).unwrap();

        assert_eq!(context.models.len(), 2);

        let staging_model = context
            .models
            .iter()
            .find(|m| m.name == "stg_events")
            .unwrap();
        // Full workspace-relative path, forward-slash normalised
        assert_eq!(
            staging_model.path.as_deref(),
            Some("models/staging/stg_events.sql")
        );
        // directory derived from path (containing directory's final component)
        assert_eq!(staging_model.directory.as_deref(), Some("staging"));

        let flat = context.models.iter().find(|m| m.name == "flat").unwrap();
        assert_eq!(flat.path.as_deref(), Some("models/flat.sql"));
        // Containing dir of models/flat.sql is "models"
        assert_eq!(flat.directory.as_deref(), Some("models"));
    }

    // D-25: find_models(directory=...) still matches on the derived directory (no regression).
    #[test]
    fn find_models_directory_filter_uses_path_derived_directory() {
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

        // SQL model in a subdirectory "staging"
        let staging_dir = project_dir.join("models").join("staging");
        std::fs::create_dir_all(&staging_dir).unwrap();
        std::fs::write(staging_dir.join("stg_orders.sql"), "SELECT 1 as order_id").unwrap();

        // Python model at the root of models/ that queries find_models(directory="staging")
        let models_dir = project_dir.join("models");
        std::fs::write(
            models_dir.join("gen.py"),
            r#"from smelt import model

@model
def combined(project):
    # D-25: directory filter should match models in "staging/"
    children = project.find_models(directory="staging")
    names = [m.name for m in children]
    if "stg_orders" in names:
        return "SELECT 'found' as result"
    return "SELECT 'not_found' as result"
"#,
        )
        .unwrap();

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
        // If directory filter works, the model found "stg_orders" and returns "found"
        assert!(
            python_models[0].content.contains("found"),
            "find_models(directory='staging') should find stg_orders; got: {}",
            python_models[0].content
        );
        assert!(!python_models[0].content.contains("not_found"));
    }
}
