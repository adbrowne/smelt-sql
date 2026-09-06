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

use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use smelt_core::config::Config;
use smelt_core::discovery::{ModelDiscovery, ModelFile, ModelKind};
use smelt_core::metadata::{extract_file_metadata, FileMetadata};
use smelt_core::python_utils::{self, ProjectContextData, ProjectModelInfo};
use smelt_core::ModelId;

#[cfg(feature = "python")]
mod embedded;
#[cfg(not(feature = "python"))]
mod subprocess;
#[cfg(test)]
mod tests;

#[cfg(feature = "python")]
use embedded::run_python_model;
#[cfg(not(feature = "python"))]
use subprocess::run_python_model;

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
//
// The `run_python_model` implementation itself lives in the feature-gated
// `subprocess` (default) or `embedded` (PyO3) submodule — see
// `crates/smelt-runtime/CLAUDE.md` "The Python model path you test depends
// on the feature set".

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
