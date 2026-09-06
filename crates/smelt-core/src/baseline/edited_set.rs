//! The workspace-comparison half of baseline materialisation: the
//! §"Attribution" edited set, derived by comparing two *loaded* workspaces
//! content-first (never `git diff --name-only`) — see `super` for the
//! module-level contract.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::discovery::ModelFile;
use crate::sources::SourceInfo;
use crate::workspace::LoadedWorkspace;

/// The §"Attribution" edited set: every model or source whose semantic
/// content differs between `base` (the baseline) and `work` (the working
/// tree), keyed by the same names `DiffGraph`'s `upstream`/`edited` use.
#[derive(Debug, Clone, Default)]
pub struct EditedSet {
    pub names: BTreeSet<String>,
    /// Project-relative paths of the files behind an edit, sorted — the
    /// JSON `edited_files` field and the text form's "N files changed"
    /// derive from this, so the two can never disagree with `names`.
    pub files: Vec<String>,
    pub project_config_changed: bool,
}

fn relative_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Bare dotted source name — `address_segments` with the leading `sources`
/// segment stripped, matching `DiffGraph::from_dependency_graph`'s
/// convention (`crates/smelt-logical/src/analysis/diff.rs`) so `edited` and
/// `upstream` key against each other.
fn source_name(info: &SourceInfo) -> String {
    match info.address_segments.split_first() {
        Some((first, rest)) if first == "sources" => rest.join("."),
        _ => info.address_segments.join("."),
    }
}

/// A `SourceInfo` compared with its (absolute, side-dependent) `path`
/// zeroed, so a field added to `SourceInfo` later is compared automatically
/// (the struct is `PartialEq`) instead of needing a hand-written field list
/// here.
fn source_without_path(info: &SourceInfo) -> SourceInfo {
    let mut cleared = info.clone();
    cleared.path = PathBuf::new();
    cleared
}

/// The §"Attribution" edited-set predicate for one model
/// (`docs/specs/property_diff.md` §"Attribution", Δ2): edited iff its
/// frontmatter-stripped SQL text differs, its parsed frontmatter metadata
/// differs, or its `smelt.yml` model override differs. A model present on
/// only one side is edited (deliberate — `diff_profiles` needs a shifted
/// downstream model to be able to attribute to an added/removed ancestor).
fn model_edited(
    base: Option<&ModelFile>,
    base_config: &crate::config::Config,
    work: Option<&ModelFile>,
    work_config: &crate::config::Config,
    name: &str,
) -> bool {
    let (base, work) = match (base, work) {
        (Some(b), Some(w)) => (b, w),
        _ => return true,
    };
    if smelt_parser::strip_frontmatter(&base.content)
        != smelt_parser::strip_frontmatter(&work.content)
    {
        return true;
    }
    if base.metadata != work.metadata {
        return true;
    }
    let base_override = base_config
        .models
        .get(name)
        .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null));
    let work_override = work_config
        .models
        .get(name)
        .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null));
    base_override != work_override
}

/// Whether a project-level `smelt.yml` key (any key other than `models`)
/// differs between the two versions.
fn project_config_changed(base: &crate::config::Config, work: &crate::config::Config) -> bool {
    fn without_models(config: &crate::config::Config) -> serde_json::Value {
        let mut value = serde_json::to_value(config).unwrap_or(serde_json::Value::Null);
        if let Some(obj) = value.as_object_mut() {
            obj.remove("models");
        }
        value
    }
    without_models(base) != without_models(work)
}

/// The §"Attribution" edited set, derived by comparing the two *loaded*
/// workspaces content-first (never `git diff --name-only`): the edited-set
/// predicates are semantic (frontmatter-stripped SQL, parsed metadata, a
/// `smelt.yml` override, a source declaration), not path-level, and
/// `DiffGraph.edited` is keyed by model/source names rather than paths.
///
/// `work` is expected to be a real `load_workspace` of the working
/// directory, so an uncommitted edit is simply content differing from the
/// archived baseline — nothing here compares two commits.
pub fn edited_set(
    base: &LoadedWorkspace,
    base_sources: &[SourceInfo],
    work: &LoadedWorkspace,
    work_sources: &[SourceInfo],
) -> EditedSet {
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut files: BTreeSet<String> = BTreeSet::new();

    let base_models: BTreeMap<String, &ModelFile> = base
        .sql_files
        .iter()
        .map(|m| (m.canonical_path(), m))
        .collect();
    let work_models: BTreeMap<String, &ModelFile> = work
        .sql_files
        .iter()
        .map(|m| (m.canonical_path(), m))
        .collect();

    let all_model_names: BTreeSet<&String> = base_models.keys().chain(work_models.keys()).collect();
    for name in all_model_names {
        let b = base_models.get(name).copied();
        let w = work_models.get(name).copied();
        if model_edited(b, &base.config, w, &work.config, name) {
            names.insert(name.clone());
            if let Some(m) = w.or(b) {
                let root = if w.is_some() {
                    &work.project_root
                } else {
                    &base.project_root
                };
                files.insert(relative_path(root, &m.path));
            }
        }
    }

    let base_sources_by_name: BTreeMap<String, SourceInfo> = base_sources
        .iter()
        .map(|s| (source_name(s), source_without_path(s)))
        .collect();
    let work_sources_by_name: BTreeMap<String, SourceInfo> = work_sources
        .iter()
        .map(|s| (source_name(s), source_without_path(s)))
        .collect();
    let base_sources_raw: BTreeMap<String, &SourceInfo> =
        base_sources.iter().map(|s| (source_name(s), s)).collect();
    let work_sources_raw: BTreeMap<String, &SourceInfo> =
        work_sources.iter().map(|s| (source_name(s), s)).collect();

    let all_source_names: BTreeSet<&String> = base_sources_by_name
        .keys()
        .chain(work_sources_by_name.keys())
        .collect();
    for name in all_source_names {
        let b = base_sources_by_name.get(name);
        let w = work_sources_by_name.get(name);
        let edited = match (b, w) {
            (Some(bi), Some(wi)) => bi != wi,
            _ => true,
        };
        if edited {
            names.insert(name.clone());
            if let Some(info) = work_sources_raw.get(name).or(base_sources_raw.get(name)) {
                let root = if work_sources_raw.contains_key(name) {
                    &work.project_root
                } else {
                    &base.project_root
                };
                files.insert(relative_path(root, &info.path));
            }
        }
    }

    EditedSet {
        names,
        files: files.into_iter().collect(),
        project_config_changed: project_config_changed(&base.config, &work.config),
    }
}
