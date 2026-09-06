//! Input gathering for the maintenance layer: resolving a model's refs to the
//! source / model-edge / clamp facts that `smelt-logical`'s pure maintenance
//! derivation reads, plus the `maintenance_plan` Salsa wrappers around it.
//!
//! Per the maintenance-plan purity rule (`CLAUDE.md` §"Maintenance-plan
//! purity"), nothing here derives a plan: every function assembles inputs and
//! calls into `smelt-logical`, or reads an already-derived verdict back.

use std::collections::HashMap;
use std::sync::Arc;

use smelt_core::metadata::{extract_file_metadata, FileMetadata};

use crate::*;

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
/// [`ref_timeseries_config`], reused by [`maintenance_plan`] to build the
/// [`smelt_logical::maintenance::SourceFacts`] the plan derivation reads.
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
/// output)" — "The output as a clocked source": "a downstream keyed model
/// may take it as its clocked driving source"). `None` when the ref does
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
fn ref_model_source_facts(
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

/// Resolve `ref_str` to an upstream **maintained-model edge**
/// (`incremental_models.md` §"Upstream model edges") when it addresses another
/// maintained (non-`full`, non-view) model in this project — `None` when the
/// ref doesn't resolve, resolves to a source/seed/function, or resolves to a
/// `full`-mode or view model (which delivers no incremental delta and so
/// contributes neither a creation cell nor a refusal). Sibling of
/// [`ref_source_info`]; reused by [`maintenance_plan_report`] to assemble the
/// [`smelt_logical::maintenance::derive::ModelEdge`]s the plan derivation
/// reads. `clock_col` is the upstream's own validated
/// `timeseries.partition_column`, or `None` when it declares none — the
/// derivation records that as a `MaintenanceReachNotDerivable` refusal.
/// Extract the addressed section's own SQL body (frontmatter stripped) and
/// [`smelt_core::metadata::ModelMetadata`] from a model file's full `text`,
/// for either a single-model file (`leaf` unused) or a multi-model file
/// (matched by declared `name:`). `None` for a generator file — its
/// maintenance metadata lives on the emitted model, not the generator
/// file's own frontmatter (not exercised by any current maintained-upstream
/// fixture; resolving it is deferred), or a file with no frontmatter.
fn resolved_model_sql_and_meta(
    text: &str,
    leaf: &str,
) -> Option<(String, smelt_core::metadata::ModelMetadata)> {
    match extract_file_metadata(text) {
        Ok(FileMetadata::Single {
            metadata,
            sql_offset,
        }) => Some((text[sql_offset..].to_string(), *metadata)),
        Ok(FileMetadata::Multi { models }) => {
            let section = models
                .into_iter()
                .find(|s| s.metadata.name.as_deref() == Some(leaf))?;
            Some((
                text[section.sql_range.clone()].to_string(),
                section.metadata,
            ))
        }
        _ => None,
    }
}

/// This model's own `smelt.sources.*` refs as [`output_delta::SourceFacts`]
/// — the per-model input the output-delta walk reads. Mirrors
/// `smelt-runtime::propagation::model_output_delta_sources`'s declared-source
/// collection over a `ModelFile`'s own `refs`, rebuilt here from `sql` text
/// via [`smelt_logical::collect_path_refs`] since the Salsa side has no
/// eagerly-loaded `ModelFile::refs` at this call site.
fn model_own_source_facts(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: Option<ProjectInput>,
    sql: &str,
) -> Vec<smelt_logical::analysis::output_delta::SourceFacts> {
    let mut sources = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for r in smelt_logical::collect_path_refs(sql) {
        let Some(stripped) = r.strip_prefix("smelt.") else {
            continue;
        };
        let Some(bare) = stripped.strip_prefix("sources.") else {
            continue;
        };
        if !seen.insert(bare.to_string()) {
            continue;
        }
        if let Some(info) = ref_source_info(db, workspace, project, &r) {
            sources.push(
                smelt_logical::analysis::output_delta::SourceFacts::from_source_info(bare, &info),
            );
        }
    }
    sources
}

/// Assemble the per-model [`smelt_logical::analysis::output_delta::
/// ModelDeltaInput`] records for the cross-model output-delta fold
/// (`derive_workspace_output_deltas`), scoped to every model transitively
/// reachable from `file`'s own refs — mirrors `smelt-runtime::propagation::
/// workspace_output_delta_verdicts`'s per-model input shape, but built by
/// walking refs rather than over an eagerly-loaded `&[ModelFile]` (`smelt-db`
/// has no such list at this call site). `address` is the ref's own
/// `smelt.`-stripped path, lowercased — the SAME key
/// [`smelt_logical::analysis::output_delta::derive_workspace_output_deltas`]
/// inserts into its verdict map, so a model-reference leaf inside any
/// reached model's own SQL resolves against it. Deduplicated by that address
/// (not by `SourceFile`), which is what makes a cyclic model-ref graph
/// terminate: each distinct address is queued at most once, so the walk is
/// bounded by the number of distinct reachable addresses regardless of how
/// many cycles connect them — never a per-model-reference recursive Salsa
/// query (`CLAUDE.md` §"Salsa purity rule"), which could not terminate over
/// a cycle.
fn model_delta_inputs(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<smelt_logical::analysis::output_delta::ModelDeltaInput> {
    let mut inputs = Vec::new();
    let mut visited: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut frontier: Vec<SourceFile> = vec![file];
    while let Some(f) = frontier.pop() {
        let text = f.text(db);
        for r in smelt_logical::collect_path_refs(text) {
            let Some(stripped) = r.strip_prefix("smelt.") else {
                continue;
            };
            let address = stripped.to_ascii_lowercase();
            if !visited.insert(address.clone()) {
                continue;
            }
            let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
            let Some(leaf) = segments.last().cloned() else {
                continue;
            };
            let Some(resolved) = resolve_ref_path(db, workspace, segments) else {
                continue;
            };
            if resolved.kind != RefKind::Model {
                continue;
            }
            let Some(model_file) = resolved.source_file else {
                continue;
            };
            let model_text = model_file.text(db);
            let Some((sql, _meta)) = resolved_model_sql_and_meta(model_text, &leaf) else {
                continue;
            };
            let project = find_project(db, workspace, model_file.project_root(db));
            let sources = model_own_source_facts(db, workspace, project, &sql);
            inputs.push(smelt_logical::analysis::output_delta::ModelDeltaInput {
                address,
                sql,
                ctx: smelt_logical::analysis::join_shape::JoinContext::new(),
                sources,
            });
            frontier.push(model_file);
        }
    }
    inputs
}

/// Every upstream maintained-model edge for `file` (`incremental_models.md`
/// §"Upstream model edges"): the model refs `file`'s own SQL makes that
/// resolve to another maintained model in this project, each carrying that
/// upstream's own validated clock and derived output-delta shape
/// (`ModelEdge::output_shape`). Its own entry point (not only inlined within
/// [`maintenance_plan_report`]) so `smelt explain`'s plan report and a
/// direct caller — a test pinning the Salsa-side derivation itself — read
/// the SAME edges rather than two independently-assembled lists. `file`
/// with no frontmatter or no `Single`-model metadata contributes no edges.
pub fn model_edges_for(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<smelt_logical::maintenance::derive::ModelEdge> {
    let text = file.text(db);
    let Ok(FileMetadata::Single { sql_offset, .. }) = extract_file_metadata(text) else {
        return Vec::new();
    };
    let sql_body = &text[sql_offset..];
    let refs = smelt_logical::collect_path_refs(sql_body);
    // The cross-model output-delta verdict map is folded ONCE per call (not
    // once per ref) over every model transitively reachable from `file`'s
    // own refs, then threaded into every `ref_model_edge` call so a
    // model-reference leaf inside any upstream's own SQL resolves against
    // it (`docs/outcomes/20260809-output-delta-typing/outcome.md` phase 9).
    let model_verdicts = smelt_logical::analysis::output_delta::derive_workspace_output_deltas(
        &model_delta_inputs(db, workspace, file),
    );
    refs.iter()
        .filter_map(|r| ref_model_edge(db, workspace, r, &model_verdicts))
        .collect()
}

fn ref_model_edge(
    db: &dyn salsa::Database,
    workspace: Workspace,
    ref_str: &str,
    model_verdicts: &std::collections::BTreeMap<
        String,
        smelt_logical::analysis::output_delta::OutputDeltaFacts,
    >,
) -> Option<smelt_logical::maintenance::derive::ModelEdge> {
    let stripped = ref_str.strip_prefix("smelt.")?;
    let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
    let leaf = segments.last()?.clone();
    let resolved = resolve_ref_path(db, workspace, segments.clone())?;
    if resolved.kind != RefKind::Model {
        return None;
    }
    let file = resolved.source_file?;
    let text = file.text(db);
    // Extract the addressed model's own `refresh:`/`timeseries:` plus its
    // own SQL body — the latter feeds `output_shape` below.
    let (sql, meta) = resolved_model_sql_and_meta(text, &leaf)?;
    // Only a maintained (`refresh: incremental`) upstream delivers an
    // incremental delta to receive; a `full`-mode or view upstream is
    // excluded (no creation cell, no refusal).
    if meta.refresh != Some(smelt_core::config::RefreshStrategy::Incremental) {
        return None;
    }
    let clock_col = meta.timeseries.as_ref().map(|t| t.partition_column.clone());
    // Sibling spellings of `clock_col` within the upstream's own SQL
    // (`ModelEdge::clock_col_aliases`'s doc comment) — derived from the same
    // `text` the metadata above was extracted from.
    let clock_col_aliases = clock_col
        .as_deref()
        .map(|c| smelt_logical::analysis::source_bounds::defining_expr_siblings(text, c))
        .unwrap_or_default();
    // The upstream's own declared top-level `unique_key:` (`models.md`
    // §"The Relation Contract"), threaded through so a downstream's P1
    // skeleton-source-closure proof over this edge can prove the join
    // one-to-one (T3, `docs/plans/20260715-composed-axes-conditional-
    // maintenance.md` Phase E3) — `ModelEdge::unique_key`'s doc comment.
    let unique_key = meta.unique_key.clone().unwrap_or_default();
    // The upstream's own derived output-delta shape (`ModelEdge::
    // output_shape`'s doc comment): the meet across whatever per-column-group
    // verdicts this upstream's own SQL derives — the SAME per-workspace fold
    // `smelt-runtime::propagation::upstream_output_delta_groups` computes,
    // never re-implemented differently here. `None` when the upstream
    // contributes no groups at all (e.g. an unclassifiable `SELECT *`
    // projection) rather than an optimistic guess.
    let project = find_project(db, workspace, file.project_root(db));
    let sources = model_own_source_facts(db, workspace, project, &sql);
    let declared_unique_key = meta.unique_key.clone().unwrap_or_default();
    let partition_col = meta.timeseries.as_ref().map(|t| t.partition_column.clone());
    let output_shape = own_output_delta_shape(
        &sql,
        &declared_unique_key,
        partition_col.as_deref(),
        &sources,
        model_verdicts,
    );
    Some(smelt_logical::maintenance::derive::ModelEdge {
        name: stripped.to_string(),
        clock_col,
        clock_col_aliases,
        unique_key,
        output_shape,
    })
}

/// A model's own derived output-delta shape: the meet across whatever
/// per-column-group verdicts its own SQL derives, given the cross-model
/// verdict map its own model-references should resolve against. Pure
/// (Salsa purity rule) — extracted out of [`ref_model_edge`] so
/// [`model_output_delta_for`] computes a model's own shape through the SAME
/// derivation a downstream's edge view of that model already uses; the two
/// call sites differ only in which model's SQL/sources/verdict-map they
/// pass in, never in what this function does with them.
fn own_output_delta_shape(
    sql: &str,
    unique_key: &[String],
    partition_col: Option<&str>,
    sources: &[smelt_logical::analysis::output_delta::SourceFacts],
    model_verdicts: &std::collections::BTreeMap<
        String,
        smelt_logical::analysis::output_delta::OutputDeltaFacts,
    >,
) -> Option<smelt_logical::analysis::output_delta::OutputDelta> {
    let skeleton =
        smelt_logical::maintenance::skeleton::skeleton_columns(sql, unique_key, partition_col);
    smelt_logical::analysis::output_delta::derive_output_delta_with_model_verdicts(
        sql,
        &smelt_logical::analysis::join_shape::JoinContext::new(),
        sources,
        &skeleton,
        model_verdicts,
    )
    .into_iter()
    .map(|(_, shape)| shape)
    .reduce(smelt_logical::analysis::output_delta::OutputDelta::meet)
}

/// This model's own emitted output-delta shape (`incremental_models.md`
/// §Surface "CLI" headline — the delta signature `smelt explain` prints
/// first): the SAME derivation [`ref_model_edge`] applies when some
/// downstream reports this model as an upstream edge, single-owned via
/// [`own_output_delta_shape`] so `smelt explain`'s own-model headline and a
/// downstream's edge view of this same model can never disagree
/// (`docs/outcomes/20260904-delta-signature-front-door/outcome.md` phase
/// 1). `None` for a generator/multi-model file (only a `Single`-model file
/// has one address to report a shape for), a file with no frontmatter, or a
/// model whose own SQL yields no output column groups.
pub fn model_output_delta_for(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<smelt_logical::analysis::output_delta::OutputDelta> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return None;
    };
    let sql = &text[sql_offset..];
    let unique_key = metadata.unique_key.clone().unwrap_or_default();
    let partition_col = metadata
        .timeseries
        .as_ref()
        .map(|t| t.partition_column.clone());
    let project = find_project(db, workspace, file.project_root(db));
    let sources = model_own_source_facts(db, workspace, project, sql);
    let model_verdicts = smelt_logical::analysis::output_delta::derive_workspace_output_deltas(
        &model_delta_inputs(db, workspace, file),
    );
    own_output_delta_shape(
        sql,
        &unique_key,
        partition_col.as_deref(),
        &sources,
        &model_verdicts,
    )
}

/// Per-source clamp observability (`docs/specs/incremental_shapes.md`
/// §"Observing the per-source clamp"): `file`'s own [`BoundResult`] per
/// `smelt.<path>` source it references, for editor hover. Thin Salsa
/// wrapper (Salsa purity rule) over the pure
/// `smelt_logical::analysis::source_bounds::derive_model_bounds`: resolves
/// each of `file`'s own refs to the upstream's declared
/// `timeseries.partition_column` (+ sibling spellings), mirroring
/// [`ref_model_edge`]'s pattern, builds the `BoundContext`, and calls the
/// pure derivation over `file`'s own SQL. Returns an empty map when `file`'s
/// own model is not itself partition-grain (no `timeseries:` declared) or
/// references no bounded sources — hover has nothing to show either way.
pub fn model_source_clamps(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> std::collections::BTreeMap<String, smelt_logical::BoundResult> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return Default::default();
    };
    if metadata.timeseries.is_none() {
        return Default::default();
    }
    let sql = &text[sql_offset..];
    let mut ctx = smelt_logical::BoundContext::new();
    for r in smelt_logical::collect_path_refs(sql) {
        let Some(stripped) = r.strip_prefix("smelt.") else {
            continue;
        };
        let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
        let Some(leaf) = segments.last().cloned() else {
            continue;
        };
        let Some(resolved) = resolve_ref_path(db, workspace, segments.clone()) else {
            continue;
        };
        if resolved.kind != RefKind::Model {
            continue;
        }
        let Some(upstream_file) = resolved.source_file else {
            continue;
        };
        let upstream_text = upstream_file.text(db);
        let Some((upstream_sql, upstream_meta)) = resolved_model_sql_and_meta(upstream_text, &leaf)
        else {
            continue;
        };
        let Some(ts) = upstream_meta.timeseries.as_ref() else {
            continue;
        };
        ctx.add_source(stripped, &ts.partition_column);
        let aliases = smelt_logical::analysis::source_bounds::defining_expr_siblings(
            &upstream_sql,
            &ts.partition_column,
        );
        ctx.add_source_partition_col_aliases(stripped, aliases);
    }
    if ctx.source_partition_cols.is_empty() {
        return Default::default();
    }
    smelt_logical::analysis::source_bounds::derive_model_bounds(sql, &ctx)
        .into_iter()
        .collect()
}

/// Thin Salsa wrapper around
/// `smelt_logical::maintenance::derive::derive_maintenance_plan`
/// (`incremental_models.md` §Surface "The plan (derived, reported)"): gathers
/// `file`'s referenced sources and declared `maintenance:`/`grain:`
/// frontmatter, then calls
/// [`crate::queries::maintenance::maintenance_plan_diagnostics`] (pure) to
/// derive the plan and map its admission refusals onto a Salsa-safe
/// return shape. Returns the default (empty) result for a model with no
/// maintenance plan (not `refresh: incremental`, or no frontmatter at all).
#[salsa::tracked]
pub fn maintenance_plan(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<crate::queries::maintenance::MaintenancePlanDiagnostics> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return Arc::new(Default::default());
    };
    let resolved_grain = metadata.resolved_grain();
    if metadata.refresh != Some(smelt_core::config::RefreshStrategy::Incremental)
        || resolved_grain.is_none()
    {
        return Arc::new(Default::default());
    }
    let path = file.path(db);
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    let sql_body = &text[sql_offset..];
    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let info = ref_source_info(db, workspace, project, r)?;
            // `SourceFacts::name` is the *bare* source name — the address
            // with the leading `sources` breadcrumb stripped
            // (`crate::maintenance::grouping` resolves a FROM alias's
            // `smelt.<path>` the same way, stripping `sources.` before
            // matching against `SourceFacts.name`; see
            // `maintenance_plan_admission.rs`'s fixtures, which name
            // sources bare — e.g. `"payments"` for `FROM
            // smelt.sources.payments`). Keeping this stripping in one place
            // (here) keeps the trigger/`scan_bounds.per_source` keys and the
            // grouping-derived `mutation_sensitivity` keys in agreement.
            let stripped = r.strip_prefix("smelt.")?;
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            Some((bare.to_string(), Some(info)))
        })
        .collect();

    let project_scan_bounds = project
        .and_then(|p| (*crate::queries::project::project_maintenance_config(db, p)).clone())
        .and_then(|m| m.scan_bounds);

    let table = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // Mirrors `maintenance_plan_report`'s own composed-driving-source
    // wiring below: a `grain: key` model's driving source may be another
    // maintained model's locality-admitted composed output, not just a
    // declared `sources:` entry.
    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let extra_model_sources: Vec<(
        smelt_logical::maintenance::SourceFacts,
        smelt_core::config::Granularity,
    )> = if resolved_grain == Some(smelt_core::config::Grain::Key) {
        refs.iter()
            .filter_map(|r| {
                ref_model_source_facts(
                    db,
                    workspace,
                    r,
                    model_scan_bounds,
                    project_scan_bounds.as_ref(),
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    // `maintenance.cells[].write` pins are validated against every one of
    // the project's declared target backends (`write_pin_diagnostics`'s own
    // doc comment) — reuses the same `project_active_backends` query the
    // `smelt.as_struct()` backend check already threads through
    // `file_diagnostics` (`as_struct_backend_diagnostics_for_file`).
    let active_backends = project
        .and_then(|p| project_active_backends(db, p))
        .unwrap_or_default();

    // `state.warehouse_tables` (`docs/specs/state.md` §"Opting out of
    // warehouse bookkeeping") — the other availability-resolution input,
    // threaded alongside `active_backends` above. Absent/unparseable config
    // resolves to the default posture (`Allowed`), same as an absent
    // `state:` block.
    let warehouse_tables = project
        .and_then(|p| project_warehouse_tables(db, p))
        .unwrap_or_default();

    // The deployed-schema snapshot (`docs/specs/definition_deltas.md`
    // §"Detection"): a Salsa world-fact input the CLI and LSP both register
    // at workspace load (`workspace_ingest::register_deployed_schemas_from_disk`).
    // `deployed_column_names` now threads the snapshot's real column names —
    // a non-skeleton `Trigger::ColumnAdded` cell that cannot be backfilled in
    // place reports `MaintenanceColumnAddNotBackfillable` as a Warning rather
    // than blocking the plan (`definition_deltas.md` §"Detection" posture
    // rules 1-3), matching what `smelt-runtime`'s own run gate already
    // admits. A model declaring `schema_evolution: strategy: full_refresh`
    // derives no definition-change trigger at all (rule 3): the runtime
    // rebuilds the whole table, so there is no in-place backfill obligation
    // to report ahead of time — implemented here, at fact assembly, rather
    // than as a new branch inside the pure derivation.
    let deployed_schema = find_deployed_schema(db, workspace, &project_root, &table);
    let deployed_model_sql: Option<String> = deployed_schema.and_then(|s| {
        s.model_sql(db)
            .as_ref()
            .map(|sql: &Arc<str>| sql.to_string())
    });
    let deployed_partition_column: Option<String> = deployed_schema.and_then(|s| {
        s.partition_column(db)
            .as_ref()
            .map(|col: &Arc<str>| col.to_string())
    });
    let full_refresh_schema_evolution = metadata.schema_evolution.as_ref().is_some_and(|se| {
        se.strategy == smelt_core::metadata::SchemaEvolutionStrategy::FullRefresh
    });
    let deployed_column_names: Vec<String> = if full_refresh_schema_evolution {
        Vec::new()
    } else {
        deployed_schema
            .map(|s| s.columns(db).iter().map(|c| c.to_string()).collect())
            .unwrap_or_default()
    };

    Arc::new(crate::queries::maintenance::maintenance_plan_diagnostics(
        sql_body,
        &table,
        &metadata,
        &source_refs,
        project_scan_bounds.as_ref(),
        &extra_model_sources,
        &active_backends,
        warehouse_tables,
        &deployed_column_names,
        deployed_model_sql.as_deref(),
        deployed_partition_column.as_deref(),
    ))
}

/// Plain (non-Salsa-tracked) counterpart of [`maintenance_plan`] that returns
/// the *full* derived plan — cells, clamps, locality verdicts — rather than
/// the Salsa-safe refusals-only projection. Used by `smelt explain <model>`
/// (`incremental_models.md` §Surface "CLI"), a one-shot CLI report that has no
/// need for Salsa's incremental caching and cannot use the tracked query
/// because [`smelt_logical::maintenance::MaintenancePlan`] does not implement
/// `PartialEq`/`Eq` (the Salsa tracked-return-value requirement the
/// refusals-only [`crate::queries::maintenance::MaintenancePlanDiagnostics`]
/// projection exists to satisfy instead).
///
/// Mirrors the exact input-assembly `maintenance_plan` performs above, but
/// calls [`crate::queries::maintenance::derive_model_maintenance_plan`]
/// directly. Still a Salsa-purity-respecting function: it only assembles
/// inputs from Salsa accessors and calls pure derivation code — it never
/// re-implements admission, locality, or ledger logic. Returns `None` for a
/// model with no maintenance plan (not `refresh: incremental`, or no
/// shape-defining fact declared and no `grain:` to resolve).
pub fn maintenance_plan_report(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<crate::queries::maintenance::MaintenancePlanResult> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return None;
    };
    let resolved_grain = metadata.resolved_grain();
    if metadata.refresh != Some(smelt_core::config::RefreshStrategy::Incremental)
        || resolved_grain.is_none()
    {
        return None;
    }
    let path = file.path(db);
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    let sql_body = &text[sql_offset..];
    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let info = ref_source_info(db, workspace, project, r)?;
            let stripped = r.strip_prefix("smelt.")?;
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            Some((bare.to_string(), Some(info)))
        })
        .collect();

    // Upstream maintained-model edges (`incremental_models.md` §"Upstream model
    // edges"): the model refs that resolve to another maintained model in
    // this project, each carrying that upstream's own validated clock and
    // derived output-delta shape.
    let model_edges = model_edges_for(db, workspace, file);

    let project_scan_bounds = project
        .and_then(|p| (*crate::queries::project::project_maintenance_config(db, p)).clone())
        .and_then(|m| m.scan_bounds);

    let table = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let (mut sources, _scan_bounds_warnings) = crate::queries::maintenance::build_source_facts(
        &source_refs,
        model_scan_bounds,
        project_scan_bounds.as_ref(),
    );
    // A `grain: key` model's driving source may itself be another
    // maintained model's locality-admitted composed output, not just a
    // declared `sources:` entry — `resolve_driving_source` (consulted
    // below via `derive_model_maintenance_plan`) is already agnostic to
    // provenance, so publish every referenced upstream model that clears
    // the locality gate into the same `SourceFacts` candidate list a
    // declared source populates (`incremental_shapes.md` §"Key temporal
    // locality (the time-partitioned output)" — "The output as a clocked
    // source"). Scoped to `grain: key` models only — a `grain: partition`
    // downstream's pushdown against a composed upstream is already derived
    // through `smelt-logical`'s own model-graph registry, not this path.
    let mut model_source_granularities: Vec<smelt_core::config::Granularity> = Vec::new();
    if resolved_grain == Some(smelt_core::config::Grain::Key) {
        for r in &refs {
            if let Some((facts, granularity)) = ref_model_source_facts(
                db,
                workspace,
                r,
                model_scan_bounds,
                project_scan_bounds.as_ref(),
            ) {
                if !sources.iter().any(|s| s.name == facts.name) {
                    sources.push(facts);
                    model_source_granularities.push(granularity);
                }
            }
        }
    }
    let key_recurrences = crate::queries::maintenance::build_key_recurrences(&source_refs);
    let explicitly_mutable: std::collections::HashSet<String> = source_refs
        .iter()
        .filter(|(_, info)| {
            info.as_ref().is_some_and(|i| {
                i.mutation_profile
                    .as_ref()
                    .is_some_and(|m| m.kind == smelt_core::sources::MutationProfile::Mutable)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();

    // The locality gate's granularity-equality structural precondition
    // needs the driving source's granularity regardless of whether it is a
    // declared source or a composed upstream model's own output — combine
    // both candidate pools and pass the union through the single shared
    // "exactly one clocked candidate, else undecided" rule
    // (`smelt_logical::maintenance::locality::single_clocked_granularity`),
    // the same rule `single_clocked_source_granularity` applies over
    // declared sources alone.
    let mut clocked_granularities: Vec<smelt_core::config::Granularity> = source_refs
        .iter()
        .filter_map(|(_, info)| info.as_ref().and_then(|i| i.timeseries.as_ref()))
        .map(|t| t.granularity)
        .collect();
    clocked_granularities.extend(model_source_granularities);
    let driving_source_granularity =
        smelt_logical::maintenance::locality::single_clocked_granularity(clocked_granularities);
    let source_referential_integrity =
        crate::queries::maintenance::build_source_referential_integrity(&source_refs);
    // The deployed-schema snapshot world-fact — see `maintenance_plan`'s own
    // call site for the full rationale: `deployed_column_names` threads the
    // snapshot's real column names (gated to empty under `schema_evolution:
    // strategy: full_refresh`, rule 3), and `model_sql` feeds the
    // skeleton-clause check; `smelt explain`'s report path reads the same
    // registered Salsa input `maintenance_plan` does.
    let deployed_schema = find_deployed_schema(db, workspace, &project_root, &table);
    let deployed_model_sql: Option<String> = deployed_schema.and_then(|s| {
        s.model_sql(db)
            .as_ref()
            .map(|sql: &Arc<str>| sql.to_string())
    });
    let deployed_partition_column: Option<String> = deployed_schema.and_then(|s| {
        s.partition_column(db)
            .as_ref()
            .map(|col: &Arc<str>| col.to_string())
    });
    let full_refresh_schema_evolution = metadata.schema_evolution.as_ref().is_some_and(|se| {
        se.strategy == smelt_core::metadata::SchemaEvolutionStrategy::FullRefresh
    });
    let deployed_column_names: Vec<String> = if full_refresh_schema_evolution {
        Vec::new()
    } else {
        deployed_schema
            .map(|s| s.columns(db).iter().map(|c| c.to_string()).collect())
            .unwrap_or_default()
    };
    let mut result = crate::queries::maintenance::derive_model_maintenance_plan_with_edges(
        sql_body,
        &table,
        &metadata,
        &sources,
        &explicitly_mutable,
        &model_edges,
        driving_source_granularity,
        &key_recurrences,
        &deployed_column_names,
        &source_referential_integrity,
        deployed_model_sql.as_deref(),
        deployed_partition_column.as_deref(),
    )?;

    // Decomposed-state summary (`docs/outcomes/20260809-rung2-state-shapes`
    // row 9): only a `grain: key` model can carry state-bearing columns
    // (`rules::cumulative::classify_cumulative` is the keyed classifier),
    // and only when it actually admits — an unadmitted model contributes an
    // empty summary rather than a guess. `classify_cumulative` is the single
    // owner of which spellings are state-bearing; this call derives nothing
    // beyond assembling its inputs from the resolved `SourceInfo`s already
    // gathered above, per the Salsa purity rule.
    if metadata.is_keyed() {
        let mut source_timeseries: smelt_logical::SourceTimeseriesMap = HashMap::new();
        for r in &refs {
            if let Some(ts) = ref_timeseries_config(db, workspace, project, r) {
                source_timeseries.insert(r.clone(), ts);
            }
        }
        if let Ok(classification) = smelt_logical::classify_cumulative(
            sql_body,
            &refs,
            &source_timeseries,
            metadata.timeseries.is_some(),
            &metadata.functional_dependencies,
        ) {
            result.state_columns = smelt_logical::state_column_summary(&classification);
            result.execution_postures = Some(classification.execution_postures());
            result.is_snapshot_reconcile = Some(classification.is_snapshot_reconcile());
        }
    }

    Some(result)
}
