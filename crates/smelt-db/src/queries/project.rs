//! Project-level queries (sources.yml, smelt.yml vars, paths/seeds/sources,
//! plus workspace-level enumeration of models / refs / sources).
//!
//! All Salsa-tracked but body delegates to `sources_config` etc. and plain
//! data structures from `smelt_core`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use smelt_types::parse_type;

use crate::config_vars;
use crate::queries::parse::parse_model;
use crate::{Model, ProjectInput, SourceFile, Workspace};

pub use smelt_types::{ModelOrigin, ModelRefValue, SourceOrigin, SourceRefValue};

use smelt_core::{SeedInfo, SourceInfo, SourcesConfig};

/// YAML parse error with location information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlParseError {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

/// Invalid type in sources.yml column definition
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTypeError {
    pub source_name: String,
    pub table_name: String,
    pub column_name: String,
    pub invalid_type: String,
}

#[salsa::tracked]
pub fn sources_config(db: &dyn salsa::Database, project: ProjectInput) -> Arc<SourcesConfig> {
    let yaml = project.sources_yaml(db);
    if yaml.is_empty() {
        return Arc::new(SourcesConfig::default());
    }
    match serde_yaml::from_str::<SourcesConfig>(yaml) {
        Ok(config) => Arc::new(config),
        Err(_) => Arc::new(SourcesConfig::default()),
    }
}

/// Return `true` when the project's `smelt.yml` contains `unstable_schema: true`.
///
/// Reads from `ProjectInput::smelt_yml_text`, which is tracked by Salsa.
/// The LSP updates the text whenever `smelt.yml` changes on disk, so this
/// query is automatically invalidated and re-evaluated on each change.
#[salsa::tracked]
pub fn project_unstable_schema(db: &dyn salsa::Database, project: ProjectInput) -> bool {
    smelt_core::parse_unstable_schema_flag(project.smelt_yml_text(db))
}

/// Return the set of *active* backend names for `project` — i.e. the
/// distinct `target_type` values in `smelt.yml`'s `targets:` map.
///
/// Phase 42: this is the set the
/// [`as_struct_backend_diagnostics_for_file`] gate intersects against
/// when a function declares `BackendSet::All` (no explicit
/// `backends:` frontmatter). Returning `None` means we could not parse
/// the workspace config — callers should treat that as "no constraint
/// on active backends" and fall back to the Phase 38 behaviour
/// (only check explicitly-declared `BackendSet::Only`).
///
/// Reads from `ProjectInput::smelt_yml_text`, which is tracked by Salsa.
#[salsa::tracked]
pub fn project_active_backends(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Option<Vec<String>> {
    smelt_core::parse_active_backends(project.smelt_yml_text(db))
}

/// Return the `vars:` block from `smelt.yml` as a `BTreeMap<String, serde_yaml::Value>`.
///
/// Returns an empty map when the `vars:` key is absent or the YAML cannot be
/// parsed. The `None` case is mapped to an empty map to avoid bubbling parse
/// errors into callers — callers that need to distinguish "vars absent" from
/// "vars present but empty" should use `config_vars::parse_vars_from_yaml` directly.
///
/// Reads from `ProjectInput::smelt_yml_text`, which is tracked by Salsa.
/// Automatically invalidated and re-evaluated whenever `smelt.yml` changes.
#[salsa::tracked]
pub fn smelt_yml_vars_query(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Arc<std::collections::BTreeMap<String, serde_yaml::Value>> {
    let text = project.smelt_yml_text(db);
    Arc::new(config_vars::parse_vars_from_yaml(text).unwrap_or_default())
}

/// Return the project's `paths:` scan-root list from `smelt.yml`, or an
/// empty list when the config text is missing or fails to parse.
///
/// Reads from `ProjectInput::smelt_yml_text` so the result is Salsa-tracked
/// and reused across every per-file resolver call. Without this, callers
/// like `resolve_ref_path` re-read and re-parse `smelt.yml` once per
/// workspace file, turning per-file diagnostics into an O(N^2) operation
/// (this was the root cause of the multi-hour CI bench regression).
///
/// The empty-on-error fallback matches the previous `Config::load(...)
/// .map(|c| c.paths).unwrap_or_default()` behaviour in the resolver: when
/// the workspace has no parseable config, no scan-root prefix is stripped
/// and the full directory path becomes part of the resolved tuple.
#[salsa::tracked]
pub fn project_paths(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<String>> {
    let text = project.smelt_yml_text(db);
    let paths = smelt_core::Config::parse_with_warnings(text)
        .map(|(c, _warnings)| c.paths)
        .unwrap_or_default();
    Arc::new(paths)
}

/// Discover seed CSV files for a project root and infer their column types.
///
/// Reads from disk (not a tracked Salsa input) — seeds that change on disk
/// require a tool restart to be detected. The query is keyed on `ProjectInput`
/// so it's recomputed when the project's sources_yaml changes, but not when
/// CSV files change.
#[salsa::tracked]
pub fn project_seeds(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<SeedInfo>> {
    let project_root = project.root(db).clone();
    let paths = smelt_core::Config::load(&project_root)
        .map(|c| c.paths)
        .unwrap_or_else(|_| vec!["models".to_string()]);
    // Phase 5: use with_sidecars so ephemeral materialization is tracked.
    Arc::new(smelt_core::discover_seed_infos_with_sidecars(
        &project_root,
        &paths,
    ))
}

/// Discover per-entity source YAML files for a project root.
///
/// Phase 6: sources live as standalone `.yml` files (no sibling `.csv`) under
/// the project's `paths:` directories. The query is keyed on `ProjectInput`
/// so it re-runs when `smelt.yml` changes (e.g. `paths:` updated) but not on
/// every source file change (LSP restarts are acceptable for source changes).
#[salsa::tracked]
pub fn project_sources(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<SourceInfo>> {
    let project_root = project.root(db).clone();
    let paths = smelt_core::Config::load(&project_root)
        .map(|c| c.paths)
        .unwrap_or_else(|_| vec!["models".to_string()]);
    Arc::new(smelt_core::discover_source_infos(&project_root, &paths))
}

#[salsa::tracked]
pub fn sources_yaml_error(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Option<YamlParseError> {
    let yaml = project.sources_yaml(db);
    if yaml.is_empty() {
        return None;
    }
    match serde_yaml::from_str::<SourcesConfig>(yaml) {
        Ok(_) => None,
        Err(e) => {
            let (line, column) = e
                .location()
                .map(|loc| (Some(loc.line()), Some(loc.column())))
                .unwrap_or((None, None));
            Some(YamlParseError {
                message: e.to_string(),
                line,
                column,
            })
        }
    }
}

#[salsa::tracked]
pub fn sources_type_errors(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Arc<Vec<SourceTypeError>> {
    let yaml = project.sources_yaml(db);
    if yaml.is_empty() {
        return Arc::new(Vec::new());
    }

    #[derive(Deserialize)]
    struct RawSourcesConfig {
        #[serde(default)]
        sources: Vec<RawSource>,
    }

    #[derive(Deserialize)]
    struct RawSource {
        name: String,
        #[serde(default)]
        tables: Vec<RawTable>,
    }

    #[derive(Deserialize)]
    struct RawTable {
        name: String,
        #[serde(default)]
        columns: Vec<RawColumn>,
    }

    #[derive(Deserialize)]
    struct RawColumn {
        name: String,
        #[serde(default, rename = "type")]
        type_str: Option<String>,
    }

    let config: RawSourcesConfig = match serde_yaml::from_str(yaml) {
        Ok(c) => c,
        Err(_) => return Arc::new(Vec::new()),
    };

    let mut errors = Vec::new();
    for source in &config.sources {
        for table in &source.tables {
            for column in &table.columns {
                if let Some(type_str) = &column.type_str {
                    if parse_type(type_str).is_err() {
                        errors.push(SourceTypeError {
                            source_name: source.name.clone(),
                            table_name: table.name.clone(),
                            column_name: column.name.clone(),
                            invalid_type: type_str.clone(),
                        });
                    }
                }
            }
        }
    }
    Arc::new(errors)
}

#[salsa::tracked]
pub fn all_models(db: &dyn salsa::Database, workspace: Workspace) -> Arc<HashMap<PathBuf, Model>> {
    let mut models = HashMap::new();
    for file in workspace.files(db).iter().copied() {
        if let Some(model) = parse_model(db, file) {
            models.insert(file.path(db).clone(), (*model).clone());
        }
    }
    Arc::new(models)
}

// ============================================================================
// Wide-reflection Salsa queries (Phase D, meta-language)
// ============================================================================
//
// Four queries materialise the `List<ModelRef>` / `List<SourceRef>` values
// that `smelt.models.with_tag`, `smelt.models.all`, `smelt.sources.with_tag`,
// and `smelt.sources.all` resolve to at expansion time.
//
// Each query is a thin wrapper over pure filtering/projection logic:
// - Read the existing `all_models` / `project_sources` Salsa input.
// - Filter by tag membership (when applicable).
// - Project to `ModelRefValue` / `SourceRefValue`.
// - Sort ascending by `path` (byte-lexicographic on workspace-relative path
//   with `/` separators).
//
// The query keys on `Workspace` / `ProjectInput` so Salsa invalidates on
// workspace-state changes that affect tag membership or file lists.
//
// Per the pure-function rule (CLAUDE.md): analysis logic is in the pure
// helpers below; queries are thin wrappers.

/// Pure helper: materialise a `ModelRefValue` from a model file.
///
/// Returns `None` when the file is not a valid model.
fn make_model_ref_value(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<ModelRefValue> {
    let model = parse_model(db, file)?;
    let raw_text = file.text(db);
    let project_root = file.project_root(db).clone();
    let abs_path = file.path(db).clone();

    // Compute workspace-relative path with `/` separators.
    let rel_path = abs_path
        .strip_prefix(&project_root)
        .unwrap_or(&abs_path)
        .to_string_lossy()
        .replace('\\', "/");

    // Extract frontmatter metadata to get the model's frontmatter tags.
    let frontmatter_metadata: Option<smelt_core::ModelMetadata> =
        smelt_core::extract_file_metadata(raw_text)
            .ok()
            .and_then(|fm| match fm {
                smelt_core::FileMetadata::Single { metadata, .. } => Some(*metadata),
                _ => None,
            });

    // Load the smelt.yml config for the project to get the merged tag set.
    let smelt_yml_text = workspace
        .projects(db)
        .iter()
        .copied()
        .find(|p| p.root(db).as_path() == project_root.as_path())
        .map(|p| p.smelt_yml_text(db).clone())
        .unwrap_or_default();

    let merged_tags =
        if let Ok((config, _)) = smelt_core::Config::parse_with_warnings(&smelt_yml_text) {
            config.get_tags(&model.name, frontmatter_metadata.as_ref())
        } else {
            // No smelt.yml or parse failure — only frontmatter tags.
            frontmatter_metadata.map(|m| m.tags).unwrap_or_default()
        };

    Some(ModelRefValue {
        path: rel_path,
        name: model.name.clone(),
        tags: merged_tags,
        model_name_for_columns: model.name.clone(),
    })
}

/// Return every model in the workspace whose merged tag set contains `tag`,
/// sorted ascending by workspace-relative `path` (byte-lexicographic, `/`
/// separators). Salsa-cached; invalidated when the workspace file list or any
/// file's content/frontmatter changes.
#[salsa::tracked]
pub fn models_with_tag(
    db: &dyn salsa::Database,
    workspace: Workspace,
    tag: String,
) -> Arc<Vec<ModelRefValue>> {
    let mut result: Vec<ModelRefValue> = workspace
        .files(db)
        .iter()
        .copied()
        .filter_map(|file| make_model_ref_value(db, workspace, file))
        .filter(|m| m.tags.contains(&tag))
        .collect();
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Arc::new(result)
}

/// Return every model in the workspace, sorted ascending by workspace-relative
/// `path` (byte-lexicographic, `/` separators). Salsa-cached; invalidated when
/// the workspace file list or any file's content changes.
#[salsa::tracked]
pub fn models_all(db: &dyn salsa::Database, workspace: Workspace) -> Arc<Vec<ModelRefValue>> {
    let mut result: Vec<ModelRefValue> = workspace
        .files(db)
        .iter()
        .copied()
        .filter_map(|file| make_model_ref_value(db, workspace, file))
        .collect();
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Arc::new(result)
}

/// Pure helper: materialise a `SourceRefValue` from a `SourceInfo`.
fn make_source_ref_value(project_root: &Path, source: &smelt_core::SourceInfo) -> SourceRefValue {
    // Workspace-relative path with `/` separators.
    let rel_path = source
        .path
        .strip_prefix(project_root)
        .unwrap_or(&source.path)
        .to_string_lossy()
        .replace('\\', "/");

    // Name: last address segment (final stem).
    let name = source.address_segments.last().cloned().unwrap_or_default();

    SourceRefValue {
        path: rel_path,
        name,
        tags: source.tags.clone(),
        address_segments: source.address_segments.clone(),
    }
}

/// Return every source in the project whose `tags:` list contains `tag`,
/// sorted ascending by workspace-relative `path` (byte-lexicographic, `/`
/// separators). Salsa-cached; invalidated when the `ProjectInput` changes.
#[salsa::tracked]
pub fn sources_with_tag(
    db: &dyn salsa::Database,
    project: ProjectInput,
    tag: String,
) -> Arc<Vec<SourceRefValue>> {
    let root = project.root(db).clone();
    let sources = project_sources(db, project);
    let mut result: Vec<SourceRefValue> = sources
        .iter()
        .filter(|s| s.tags.contains(&tag))
        .map(|s| make_source_ref_value(&root, s))
        .collect();
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Arc::new(result)
}

/// Return every source in the project, sorted ascending by workspace-relative
/// `path` (byte-lexicographic, `/` separators). Salsa-cached; invalidated
/// when the `ProjectInput` changes.
#[salsa::tracked]
pub fn sources_all(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<SourceRefValue>> {
    let root = project.root(db).clone();
    let sources = project_sources(db, project);
    let mut result: Vec<SourceRefValue> = sources
        .iter()
        .map(|s| make_source_ref_value(&root, s))
        .collect();
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Arc::new(result)
}
