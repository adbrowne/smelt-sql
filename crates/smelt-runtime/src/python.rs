//! Python model discovery and execution — the shared runtime implementation.
//!
//! This module was migrated from `smelt-cli/src/python.rs` so that both the CLI
//! and UI consume the same discovery logic (Run Pipeline Parity rule). The CLI
//! keeps a thin re-export wrapper; the UI calls this directly.
//!
//! Python models are the "escape hatch" — deliberately low-level, for the ~5% of
//! cases where you need programmatic model generation (e.g., dynamically union all
//! models with a tag). Python models return SQL strings that get parsed by the
//! existing smelt parser.

#[cfg(not(feature = "python"))]
use anyhow::Context;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(feature = "python"))]
use std::process::{Command, Stdio};

use smelt_core::config::Config;
use smelt_core::discovery::{ModelDiscovery, ModelFile, ModelKind};
use smelt_core::metadata::{extract_file_metadata, FileMetadata};
use smelt_core::python_utils::{self, ProjectContextData, ProjectModelInfo};
use smelt_core::ModelId;

/// Output from a single Python model function.
#[derive(Debug, Deserialize)]
struct PythonModelOutput {
    name: String,
    sql: String,
    #[serde(default)]
    queries: Vec<smelt_core::PythonModelQuery>,
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
        return Err(anyhow!(
            "Python model error in {}:\n{}",
            file_path.display(),
            stderr
        ));
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
                anyhow!(
                    "Python model error in {}:\n{}{}",
                    file_path.display(),
                    tb,
                    e
                )
            })?;

        Ok(outputs
            .into_iter()
            .map(|o| PythonModelOutput {
                name: o.name,
                sql: o.sql,
                queries: o.queries,
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
) -> Result<String> {
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
    serde_json::to_string(&context)
        .map_err(|e| anyhow!("Failed to serialize project context: {}", e))
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
    let python = python_utils::find_python(config_python).ok_or_else(|| {
        anyhow!(
            "Python interpreter not found.\n\
             Install Python 3 or set the SMELT_PYTHON environment variable."
        )
    })?;
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
        let context_json = build_project_context(sql_models, &python_models, config, project_dir)?;
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
                    smelt_core::extract_refs(&file)
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
                    let mut segs = ModelDiscovery::compute_address_segments(
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

/// Re-export `PythonModelQuery` so consumers of this module get the canonical type.
pub use smelt_core::PythonModelQuery;

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::config::Materialization;

    /// Helper: find and copy the Python SDK into a temp dir.
    fn setup_sdk(project_dir: &std::path::Path) {
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
    }

    fn minimal_config() -> Config {
        Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
            state: Default::default(),
        }
    }

    // ── Pure unit tests (no Python interpreter needed) ────────────────────────

    #[test]
    fn normalize_python_sql_section_delimiter() {
        // D-22: `--- name: X ---` is rewritten to plain `---\nname: X` frontmatter.
        let input = "--- name: my_model ---\nmaterialization: table\n---\nSELECT 1";
        let result = normalize_python_sql(input);
        assert!(result.starts_with("---\nname: my_model"));
    }

    #[test]
    fn normalize_python_sql_plain_frontmatter_unchanged() {
        let input = "---\nmaterialization: table\n---\nSELECT 1";
        let result = normalize_python_sql(input);
        assert_eq!(result.as_ref(), input);
    }

    #[test]
    fn normalize_python_sql_no_frontmatter_unchanged() {
        let input = "SELECT 1 AS id";
        let result = normalize_python_sql(input);
        assert_eq!(result.as_ref(), input);
    }

    #[test]
    fn models_equal_order_independent() {
        let model_a = ModelFile {
            name: "alpha".to_string(),
            path: std::path::PathBuf::from("a.py"),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(std::path::PathBuf::from("a.py")),
            address_segments: Vec::new(),
        };
        let model_b = ModelFile {
            name: "beta".to_string(),
            path: std::path::PathBuf::from("b.py"),
            content: "SELECT 2".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(std::path::PathBuf::from("b.py")),
            address_segments: Vec::new(),
        };
        let set1 = vec![model_a.clone(), model_b.clone()];
        let set2 = vec![model_b, model_a];
        assert!(models_equal(&set1, &set2));
    }

    #[test]
    fn models_equal_different_content() {
        let model_a = ModelFile {
            name: "same".to_string(),
            path: std::path::PathBuf::from("a.py"),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(std::path::PathBuf::from("a.py")),
            address_segments: Vec::new(),
        };
        let model_b = ModelFile {
            name: "same".to_string(),
            path: std::path::PathBuf::from("a.py"),
            content: "SELECT 2".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(std::path::PathBuf::from("a.py")),
            address_segments: Vec::new(),
        };
        assert!(!models_equal(&[model_a], &[model_b]));
    }

    // D-26: path-derived address segments (pure unit test, no Python).
    #[test]
    fn python_address_is_path_derived() {
        let root = std::path::PathBuf::from("/project");
        let paths = vec!["py".to_string()];

        // archive.py in py/ → ["archive"] + function name "users" → ["archive", "users"]
        let file = root.join("py").join("archive.py");
        let mut segs = ModelDiscovery::compute_address_segments(&file, &root, &paths);
        segs.push("users".to_string());
        assert_eq!(segs, vec!["archive", "users"]);

        // py/util.py + function "helper" → ["util", "helper"]
        let file2 = root.join("py").join("util.py");
        let mut segs2 = ModelDiscovery::compute_address_segments(&file2, &root, &paths);
        segs2.push("helper".to_string());
        assert_eq!(segs2, vec!["util", "helper"]);

        // subdirectory: py/staging/stg.py + function "events" → ["staging", "stg", "events"]
        let file3 = root.join("py").join("staging").join("stg.py");
        let mut segs3 = ModelDiscovery::compute_address_segments(&file3, &root, &paths);
        segs3.push("events".to_string());
        assert_eq!(segs3, vec!["staging", "stg", "events"]);
    }

    // D-26: collision between Python and SQL model with same path-derived address.
    #[test]
    fn python_sql_address_collision_is_duplicate_address() {
        use smelt_core::resolver::resolve_address_map;

        let sql_model = ModelFile {
            name: "users".to_string(),
            path: std::path::PathBuf::from("/project/py/archive.sql"),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Sql,
            model_id: ModelId::from_path(std::path::PathBuf::from("/project/py/archive.sql")),
            address_segments: vec!["archive".to_string(), "users".to_string()],
        };
        let python_model = ModelFile {
            name: "users".to_string(),
            path: std::path::PathBuf::from("/project/py/archive.py"),
            content: "SELECT 2".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: ModelKind::Python {
                source_line: 1,
                queries: vec![],
            },
            model_id: ModelId::from_path(std::path::PathBuf::from("/project/py/archive.py")),
            address_segments: vec!["archive".to_string(), "users".to_string()],
        };

        let all_models = vec![sql_model, python_model];
        let (_map, collisions) = resolve_address_map(&all_models, &[], &[]);
        assert_eq!(
            collisions.len(),
            1,
            "same path-derived address must produce a collision"
        );
        assert_eq!(collisions[0].address, vec!["archive", "users"]);
    }

    // ── Integration tests (require Python interpreter + SDK) ─────────────────

    /// `python_discovery_runs_in_runtime` — TDD anchor for P1: verify the moved
    /// `discover_python_models` produces the same `ModelFile` set in runtime as
    /// the CLI did. Uses the subprocess path (no PyO3 required).
    #[test]
    fn python_discovery_runs_in_runtime() {
        let tmp = tempfile::TempDir::new().unwrap();
        let project_dir = tmp.path();
        setup_sdk(project_dir);

        // A minimal SQL model (smelt-core's discover_models errors on empty projects)
        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("anchor.sql"), "SELECT 1 AS anchor_id").unwrap();

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

        let discovery = ModelDiscovery::new(project_dir.to_path_buf(), vec!["models".to_string()]);
        let sql_models = discovery.discover_models().unwrap();
        let python_files = discovery.discover_python_files().unwrap();
        let config = minimal_config();

        let python_models =
            discover_python_models(&python_files, &sql_models, &config, project_dir, None).unwrap();

        assert_eq!(
            python_models.len(),
            1,
            "runtime must discover the Python model"
        );
        assert_eq!(python_models[0].name, "dynamic_model");
        assert!(python_models[0].content.contains("SELECT 1"));
        // D-26: path-derived address segments
        assert_eq!(
            python_models[0].address_segments,
            vec!["gen", "dynamic_model"]
        );
    }

    /// D-22: Python output with `--- name: X ---` section-delimiter format where X
    /// matches the function name must be treated as single-model — no mismatch error.
    #[test]
    fn python_multimodel_delimiter_not_a_section() {
        let tmp = tempfile::TempDir::new().unwrap();
        let project_dir = tmp.path();
        setup_sdk(project_dir);

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("anchor.sql"), "SELECT 1 AS anchor_id").unwrap();

        std::fs::write(
            models_dir.join("matching_func.py"),
            r#"from smelt import model

@model
def matching_func(project):
    return """--- name: matching_func ---
materialization: table
---
SELECT 1 AS id
"""
"#,
        )
        .unwrap();

        let discovery = ModelDiscovery::new(project_dir.to_path_buf(), vec!["models".to_string()]);
        let python_files = discovery.discover_python_files().unwrap();
        let config = minimal_config();

        let python_models =
            discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();

        assert_eq!(
            python_models.len(),
            1,
            "section-delimiter must produce exactly 1 model (D-22)"
        );
        let model = &python_models[0];
        assert_eq!(model.name, "matching_func");
        assert!(
            model.parse_errors.is_empty(),
            "matching name must produce no errors; got: {:#?}",
            model.parse_errors
        );
        let meta = model.metadata.as_ref().expect("metadata must be populated");
        assert_eq!(meta.materialization, Some(Materialization::Table));
    }

    /// D-27: Python name mismatch must block the build AND retain other frontmatter keys.
    #[test]
    fn python_name_mismatch_blocks_and_retains_other_keys() {
        let tmp = tempfile::TempDir::new().unwrap();
        let project_dir = tmp.path();
        setup_sdk(project_dir);

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("anchor.sql"), "SELECT 1 AS anchor_id").unwrap();

        std::fs::write(
            models_dir.join("mismatch.py"),
            r#"from smelt import model

@model
def my_func(project):
    return """--- name: other_name ---
materialization: table
---
SELECT 1 AS id
"""
"#,
        )
        .unwrap();

        let discovery = ModelDiscovery::new(project_dir.to_path_buf(), vec!["models".to_string()]);
        let python_files = discovery.discover_python_files().unwrap();
        let config = minimal_config();

        let python_models =
            discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 1, "model must still be produced");
        let model = &python_models[0];
        assert_eq!(model.name, "my_func");

        // PythonModelNameMismatch diagnostic must be present
        let mismatch_errs: Vec<_> = model
            .parse_errors
            .iter()
            .filter(|e| e.message.starts_with("PythonModelNameMismatch"))
            .collect();
        assert_eq!(
            mismatch_errs.len(),
            1,
            "expected exactly one PythonModelNameMismatch"
        );
        let msg = &mismatch_errs[0].message;
        assert!(msg.contains("other_name"), "must mention frontmatter name");
        assert!(msg.contains("my_func"), "must mention function name");

        // D-27: other frontmatter keys (materialization) must be RETAINED
        let meta = model
            .metadata
            .as_ref()
            .expect("metadata must be retained (D-27)");
        assert_eq!(meta.materialization, Some(Materialization::Table));
        assert!(meta.name.is_none(), "name: field must be stripped");
    }

    /// Non-convergent Python model (output changes every round) must produce an error.
    #[test]
    fn non_convergent_set_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        let project_dir = tmp.path();
        setup_sdk(project_dir);

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

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

        let python_files = vec![(
            models_dir.join("unstable.py"),
            vec![5u32],
            py_content.clone(),
        )];
        let config = minimal_config();

        let result = discover_python_models(&python_files, &[], &config, project_dir, None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("converge") || err_msg.contains("circular"),
            "expected non-convergence error, got: {err_msg}"
        );
    }
}
