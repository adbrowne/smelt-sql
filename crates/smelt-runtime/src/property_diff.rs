//! The shared property-diff pipeline (`docs/specs/property_diff.md`), split
//! into three steps so both the CLI (`smelt explain --diff`) and the editor
//! (`docs/outcomes/20260905-property-diff/phases/07-plan.md`) derive from
//! ONE implementation — the editor never runs its own comparison
//! (§Constraints item 5).
//!
//! `work_side` loads and derives the working tree; `baseline_side` resolves
//! and materialises a git baseline and derives it; `report` diffs the two
//! and assembles the presentation envelope. The CLI calls all three in that
//! order (D2 of the phase 5 plan: a broken working tree fails before any
//! scratch directory exists). The LSP calls `work_side` once per refresh and
//! caches `baseline_side`'s result keyed on the resolved commit
//! (`docs/outcomes/20260905-property-diff/phases/07-plan.md` D2), since it
//! is the more expensive of the two sides (a `git archive` plus a second
//! workspace load).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smelt_core::baseline::{
    edited_set, materialize, resolve_baseline, BaselineError, EditedSet, ResolvedBaseline,
};
use smelt_core::graph::DependencyGraph;
use smelt_core::sources::SourceInfo;
use smelt_core::workspace::{apply_open_buffers, load_workspace, LoadedWorkspace};
use smelt_logical::analysis::diff::{
    apply_failure_reasons, diff_profiles, BaselineInfo, DiffGraph, DiffReport,
};

use crate::profile::{profiles_for_workspace, WorkspaceProfiles};

/// Errors the shared pipeline can produce. A thin wrapper over
/// [`BaselineError`] plus the graph-build failure the CLI already surfaced
/// as an `anyhow` context string — kept as a `String` here rather than a
/// new variant because `DependencyGraph::build`'s error type is not
/// `std::error::Error` (see its own definition), and this crate's
/// `pub(crate)`-only compile internals rule does not extend to error
/// plumbing for a `pub` module like this one.
#[derive(Debug, thiserror::Error)]
pub enum PropertyDiffError {
    #[error(transparent)]
    Baseline(#[from] BaselineError),
    #[error("failed to derive property profiles: {0}")]
    Profile(String),
    #[error("failed to build dependency graph: {0}")]
    Graph(String),
}

/// The working-tree side of a property diff.
pub struct WorkSide {
    pub loaded: LoadedWorkspace,
    pub sources: Vec<SourceInfo>,
    pub profiles: WorkspaceProfiles,
    pub graph: DependencyGraph,
}

/// The baseline side of a property diff: a resolved-and-materialised git
/// ref plus its derived profiles. This is the side an editor caches, keyed
/// on `resolved.commit` (`docs/outcomes/20260905-property-diff/phases/
/// 07-plan.md` D2) — `resolved` and `profiles` outlive the scratch
/// directory `materialize` created (its `BaselineCheckout` is created and
/// dropped entirely inside `baseline_side`, per Constraint 8: nothing here
/// reads from the scratch path after this function returns).
pub struct BaselineSide {
    pub resolved: ResolvedBaseline,
    pub loaded: LoadedWorkspace,
    pub sources: Vec<SourceInfo>,
    pub profiles: WorkspaceProfiles,
}

/// Load and derive the working tree's property profiles.
///
/// `overlays` carries open editor buffers for tracked `.sql` model paths
/// (`docs/outcomes/20260905-property-diff/phases/07-plan.md` D4); the CLI
/// passes an empty map, since it always reads from disk.
pub fn work_side(
    project_dir: &Path,
    overlays: &BTreeMap<PathBuf, String>,
) -> Result<WorkSide, PropertyDiffError> {
    let mut loaded = load_workspace(project_dir);
    apply_open_buffers(&mut loaded, overlays);
    let profiles =
        profiles_for_workspace(&loaded).map_err(|e| PropertyDiffError::Profile(e.to_string()))?;
    let sources = smelt_core::discover_source_infos(&loaded.project_root, &loaded.config.paths);
    let legacy_sources = smelt_cli_sources_config(project_dir);
    let graph = DependencyGraph::build(loaded.sql_files.clone(), legacy_sources.as_ref())
        .map_err(|e| PropertyDiffError::Graph(format!("{e:?}")))?;
    Ok(WorkSide {
        loaded,
        sources,
        profiles,
        graph,
    })
}

/// `SourcesConfig::load` lives in `smelt-core`, re-exported by `smelt-cli`
/// as a convenience; called directly here so this module has no dependency
/// on `smelt-cli` (which itself depends on `smelt-runtime` — a dependency
/// back-edge this module must not introduce).
fn smelt_cli_sources_config(project_dir: &Path) -> Option<smelt_core::sources::SourcesConfig> {
    smelt_core::sources::SourcesConfig::load(project_dir).ok()
}

/// Resolve, materialise, and derive the baseline side.
///
/// `explicit_ref` is `None` to default to the merge-base baseline. The
/// scratch checkout is created and dropped entirely inside this call —
/// everything derived from it is read before it returns (Constraint 8, "no
/// repository mutation" plus honest cleanup).
pub fn baseline_side(
    project_dir: &Path,
    explicit_ref: Option<&str>,
) -> Result<BaselineSide, PropertyDiffError> {
    let resolved = resolve_baseline(project_dir, explicit_ref)?;
    let checkout = materialize(&resolved)?;
    let loaded = load_workspace(checkout.project_root());
    let profiles =
        profiles_for_workspace(&loaded).map_err(|e| PropertyDiffError::Profile(e.to_string()))?;
    let sources = smelt_core::discover_source_infos(&loaded.project_root, &loaded.config.paths);
    drop(checkout);
    Ok(BaselineSide {
        resolved,
        loaded,
        sources,
        profiles,
    })
}

/// Diff `work` against `base` and assemble the [`DiffReport`] envelope.
///
/// The working-tree graph is the one attribution walks
/// (`docs/specs/property_diff.md` §"Attribution").
pub fn report(work: &WorkSide, base: &BaselineSide) -> DiffReport {
    let EditedSet {
        names,
        project_config_changed,
        files,
    } = edited_set(&base.loaded, &base.sources, &work.loaded, &work.sources);

    let diff_graph = DiffGraph::from_dependency_graph(&work.graph, names, project_config_changed);

    let mut diff = diff_profiles(
        &base.profiles.profiles,
        &work.profiles.profiles,
        &diff_graph,
    );
    apply_failure_reasons(&mut diff, &base.profiles.failures, &work.profiles.failures);

    let baseline_info = BaselineInfo::from(&base.resolved);
    DiffReport::new(baseline_info, files, diff)
}
