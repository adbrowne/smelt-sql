use smelt_core::metadata::{extract_file_metadata, FileMetadata};

use crate::*;

use super::plan::maintenance_plan_report;

/// Resolve a `smelt.<path>` ref string to its definition's frontmatter
/// `timeseries:` block, when it resolves to a model that declares one. This
/// reconstructs (project-scoped) the `smelt.<path> → timeseries` lookup the
/// runtime builds from the model graph, so the keyed classifier sees the
/// same driving sources in the editor as it does at build time.
pub(crate) fn ref_timeseries_config(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: Option<ProjectInput>,
    ref_str: &str,
) -> Option<smelt_core::config::TimeseriesConfig> {
    let segments: Vec<String> = ref_str
        .strip_prefix("smelt.")?
        .split('.')
        .map(|s| s.to_string())
        .collect();
    let leaf = segments.last()?.clone();
    let resolved = resolve_ref_path(db, workspace, segments.clone())?;
    // Per-entity source YAML (`RefKind::Source`) has no `source_file` — its
    // `timeseries:` block lives on the `SourceInfo` the project's source scan
    // already parsed, not on a frontmatter-bearing model file. Look it up by
    // `address_segments` before falling through to the model-file path below
    // (which only applies to `RefKind::Model`/generator refs).
    if resolved.kind == RefKind::Source {
        let project = project?;
        return project_sources(db, project)
            .iter()
            .find(|s| s.address_segments == segments)
            .and_then(|s| s.timeseries.clone());
    }
    let file = resolved.source_file?;
    let text = file.text(db);
    match extract_file_metadata(text) {
        // Hand-authored single model: the `timeseries:` is its own frontmatter.
        Ok(FileMetadata::Single { metadata, .. }) => metadata.timeseries.clone(),
        // Multi-model file: match the addressed section by name.
        Ok(FileMetadata::Multi { models }) => models
            .iter()
            .find(|s| s.metadata.name.as_deref() == Some(leaf.as_str()))
            .and_then(|s| s.metadata.timeseries.clone()),
        // Generator-emitted model: `timeseries:` is inherited onto the emitted
        // model (carried on the `EmittedModelDef`), not on the generator file's
        // own frontmatter — mirror the runtime, which reads it from the graph.
        Ok(FileMetadata::Generator { .. }) => emitted_models(db, workspace)
            .survivors
            .iter()
            .find(|e| e.name == leaf)
            .and_then(|e| e.timeseries_config.clone()),
        _ => None,
    }
}

/// Resolve `ref_str` to its [`smelt_core::SourceInfo`] when it addresses a
/// declared source — `None` when the ref doesn't resolve, or resolves to
/// something other than a source (a model, seed, function). Sibling of
/// [`ref_timeseries_config`], reused by [`crate::maintenance_refs::maintenance_plan`]
/// to build the [`smelt_logical::maintenance::SourceFacts`] the plan derivation reads.
pub(crate) fn ref_source_info(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: Option<ProjectInput>,
    ref_str: &str,
) -> Option<smelt_core::SourceInfo> {
    let segments: Vec<String> = ref_str
        .strip_prefix("smelt.")?
        .split('.')
        .map(|s| s.to_string())
        .collect();
    let resolved = resolve_ref_path(db, workspace, segments.clone())?;
    if resolved.kind != RefKind::Source {
        return None;
    }
    let project = project?;
    project_sources(db, project)
        .iter()
        .find(|s| s.address_segments == segments)
        .cloned()
}

/// Resolve `ref_str` to a locality-admitted composed model's own output as
/// a [`smelt_logical::maintenance::SourceFacts`] candidate driving source
/// (`incremental_shapes.md` §"Key temporal locality (the time-partitioned
/// output)" — "The output as a clocked source"). `None` when the ref does
/// not resolve to a maintained `grain: key` model whose own `timeseries:`
/// block cleared the locality gate — a declared source, a `full`/view
/// model, a `grain: partition` model (already visible to downstream
/// pushdown via `smelt-logical`'s own model-graph registry, not this
/// path), or a keyed model whose own locality gate refused all resolve to
/// `None` here, so the caller's driving-source resolution falls back to
/// whatever declared sources it has.
///
/// Recurses one level into [`maintenance_plan_report`] over the upstream's
/// own file to read its already-derived
/// [`smelt_logical::maintenance::KeyLocality`] verdict — this never
/// re-implements the locality gate itself (`CLAUDE.md` §"Maintenance-plan
/// purity"): it calls the same pure entry point
/// ([`smelt_logical::maintenance::locality::establish_locality`], reached
/// via [`crate::queries::maintenance::derive_model_maintenance_plan`]) the
/// upstream's own plan derivation already calls, and reads its result
/// rather than deriving a second one. Terminates because the model graph
/// is acyclic (a `smelt.ref()` cycle is rejected elsewhere in workspace
/// loading); a long composed chain recurses one frame per hop, which is
/// how the clock is meant to propagate through the DAG.
/// Returns the candidate [`SourceFacts`](smelt_logical::maintenance::SourceFacts)
/// alongside the upstream's own declared `timeseries.granularity` — a
/// downstream keyed model's locality gate needs both: the source-shape
/// candidate for [`smelt_logical::maintenance::locality::
/// resolve_driving_source`], and the granularity for the gate's
/// granularity-equality structural precondition (mirroring
/// [`crate::queries::maintenance::single_clocked_source_granularity`]'s
/// role for declared sources).
/// `model_scan_bounds`/`project_scan_bounds` are the DOWNSTREAM (referencing)
/// model's own `maintenance.scan_bounds` declarations — the same two configs
/// [`crate::queries::maintenance::build_source_facts`] already threads for a
/// declared `sources:` entry — consulted here so a model-edge candidate can
/// be granted `allow_full_scan` too (keyed by its bare, `smelt.`-stripped
/// name, exactly like a declared source's `per_source` key): before this,
/// there was no way to declare the K8 escape hatch for an upstream
/// maintained-model source at all, which phase 19
/// (`docs/outcomes/20260815-definition-delta-migrate`) newly needs — an
/// `UpstreamMutation` cell is now genuinely derivable for one of these
/// candidates too (an `AppendOnly` composed source in a value-sensitive
/// aggregate column group), not only for a declared `sources:` entry.
pub(super) fn ref_model_source_facts(
    db: &dyn salsa::Database,
    workspace: Workspace,
    ref_str: &str,
    model_scan_bounds: Option<&smelt_core::config::ScanBoundsConfig>,
    project_scan_bounds: Option<&smelt_core::config::ScanBoundsConfig>,
) -> Option<(
    smelt_logical::maintenance::SourceFacts,
    smelt_core::config::Granularity,
)> {
    let stripped = ref_str.strip_prefix("smelt.")?;
    let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
    let resolved = resolve_ref_path(db, workspace, segments.clone())?;
    if resolved.kind != RefKind::Model {
        return None;
    }
    let file = resolved.source_file?;
    let result = maintenance_plan_report(db, workspace, file)?;
    let locality = result.plan.key_locality.as_ref()?;
    let granularity = ref_timeseries_config(
        db,
        workspace,
        find_project(db, workspace, file.project_root(db)),
        ref_str,
    )?
    .granularity;
    let (allow_full_scan, _require, _on_violation) =
        crate::queries::maintenance::effective_scan_bounds(
            stripped,
            model_scan_bounds,
            project_scan_bounds,
        );
    Some((
        smelt_logical::maintenance::SourceFacts {
            name: stripped.to_string(),
            // A composed maintained output's rows, once written by a run,
            // are not retroactively mutated by a *later* run touching a
            // different slice — the same append-only posture a declared
            // `timeseries:` source with no explicit
            // `mutation_profile: mutable` gets by default
            // (`crate::queries::maintenance::source_facts`).
            mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
            partition_col: Some(locality.slice.partition_column().to_string()),
            unique_key: Vec::new(),
            allow_full_scan,
        },
        granularity,
    ))
}
