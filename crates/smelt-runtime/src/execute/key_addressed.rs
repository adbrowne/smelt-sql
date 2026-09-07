use super::retry::*;

use std::collections::{HashMap, HashSet};
use std::time::Duration as StdDuration;

use anyhow::Result;

use smelt_backend::Backend;
use smelt_core::config::Config;
use smelt_state::file_store::FileStore;

use crate::reporter::RunReporter;
use crate::types::ExecuteRequest;
use crate::{EphemeralResolver, SqlCompiler};

/// Build `model_file`'s upstream **maintained-model** edge list
/// (`docs/specs/incremental_models.md` §"Upstream model edges") — the input
/// T3 delta restriction (`docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase E3) needs to attempt restricting a model-edge-
/// sourced creation cell's recompute. Mirrors `crate::propagation::
/// derive_clamp_and_locality`'s own model-edge extraction exactly (that
/// module's already-shipped precedent for this same shape): a raw
/// `sources.*` ref contributes no edge (that's `maint_source_facts`'/
/// `SourceFacts`' job, built separately), and a ref this workspace does not
/// resolve to another model at all — or resolves to one whose own
/// `refresh:` is not `incremental` (a `full`-mode or view upstream delivers
/// no incremental delta) — contributes no edge either, never a spurious
/// permissive whole-table synthesis.
pub(crate) fn model_edges_for(
    model_file: &smelt_core::ModelFile,
    model_by_addr: &HashMap<String, smelt_core::ModelFile>,
    source_infos: &[smelt_core::sources::SourceInfo],
) -> Vec<smelt_logical::maintenance::derive::ModelEdge> {
    let mut edges = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // The SAME per-workspace output-delta fold `crate::propagation::
    // build_forward_graph`'s own `type_edge` call reads — never a second,
    // independent derivation (`docs/outcomes/20260809-output-delta-typing/
    // phases/06-summary.md` "For the next planner"). Recomputed per call
    // (this driver-facing resolver has no cross-model cache to lean on, and
    // is only ever called once per live model per run); the workspace-wide
    // fixed-point fold itself is O(models) per model reference resolved.
    let model_by_addr_ref: std::collections::BTreeMap<String, &smelt_core::ModelFile> =
        model_by_addr.iter().map(|(k, v)| (k.clone(), v)).collect();
    let models: Vec<smelt_core::ModelFile> = model_by_addr.values().cloned().collect();
    let workspace_verdicts =
        crate::propagation::workspace_output_delta_verdicts(&models, source_infos);
    for r in &model_file.refs {
        let segs = r.smelt_ref.to_path();
        if segs.first().map(|s| s.as_str()) == Some("sources") {
            continue;
        }
        let addr = segs.join(".");
        if !seen.insert(addr.clone()) {
            continue;
        }
        let Some(upstream) = model_by_addr.get(&addr) else {
            continue;
        };
        let up_meta = upstream.metadata.as_deref();
        let is_maintained = up_meta
            .map(|m| m.refresh == Some(smelt_core::config::RefreshStrategy::Incremental))
            == Some(true);
        if !is_maintained {
            continue;
        }
        let clock_col = up_meta
            .and_then(|m| m.timeseries.as_ref())
            .map(|ts| ts.partition_column.clone());
        // Sibling spellings of `clock_col` within the upstream's own SQL
        // (`ModelEdge::clock_col_aliases`'s doc comment).
        let clock_col_aliases = clock_col
            .as_deref()
            .map(|c| {
                smelt_logical::analysis::source_bounds::defining_expr_siblings(&upstream.content, c)
            })
            .unwrap_or_default();
        let unique_key = up_meta
            .and_then(|m| m.unique_key.clone())
            .unwrap_or_default();
        // The upstream's own derived output-delta shape — the meet across
        // whatever per-column-group verdicts `upstream_output_delta_groups`
        // derives for it, mirroring `propagation.rs`'s own `ModelEdge`
        // construction exactly.
        let output_shape = crate::propagation::upstream_output_delta_groups(
            &addr,
            &model_by_addr_ref,
            source_infos,
            &workspace_verdicts,
        )
        .into_iter()
        .map(|(_, shape)| shape)
        .reduce(smelt_logical::analysis::output_delta::OutputDelta::meet);
        edges.push(smelt_logical::maintenance::derive::ModelEdge {
            name: addr,
            clock_col,
            clock_col_aliases,
            unique_key,
            output_shape,
        });
    }
    edges
}

/// Outcome of a live key-addressed model-edge repair cell
/// ([`resolve_and_dispatch_key_addressed_edge_cell`]) that actually executed
/// a write this run (an empty changed-key set resolves to `Ok(None)` from
/// the underlying dispatch and is reported by the caller as the ordinary
/// zero-row no-op, not this variant).
pub(crate) struct KeyAddressedEdgeDispatch {
    pub(crate) result: smelt_backend::ExecutionResult,
    pub(crate) used_per_group_recompute: bool,
    pub(crate) used_diff_patch: bool,
    /// The upstream model's bare name the cell is keyed on — used by callers
    /// to assert mutual exclusion against a declared-source cell resolved
    /// for the same trigger name.
    pub(crate) edge_name: String,
}

/// Resolve and (if live, and the target table already exists) dispatch a
/// key-addressed model-edge repair cell (`docs/specs/incremental_models.md`
/// §"Upstream model edges") — the SAME resolve-then-execute body the keyed
/// run branch and the non-keyed (window-forward) incremental branch both
/// need, factored out here so the two cannot silently diverge the way they
/// did before this cell was dispatched on both branches
/// (`docs/outcomes/20260815-definition-delta-migrate/phases/11-plan.md`).
///
/// The cell has no run-window axis of its own — its bounded read is the
/// upstream's own affected key set, not a `[start, end)` interval — so it
/// dispatches identically regardless of which run shape the downstream's
/// OWN driving trigger classifies as, and regardless of the downstream's
/// declared `grain:`. Never dispatched on the creation run: `table_exists_
/// before_run` must be captured by the caller BEFORE any write this run
/// performs, since there is nothing to repair yet and the fold/batch loop's
/// own create path is what materializes the table.
/// Mutation-happened discrimination
/// (`docs/specs/incremental_models.md` §"When a mutation cell dispatches"):
/// resolve `source`'s `SourceInfo` (same bare-address lookup every
/// `UpstreamMutation` dispatch site already performs), and — only if it
/// declares digest columns — probe its current whole-source fingerprint
/// against the recorded baseline. Returns `None` when the source has no
/// declared columns to fingerprint (nothing to compare against, so the
/// caller treats it the same as `Dispatch`) or is not found in
/// `source_infos` at all.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_upstream_mutation_gate(
    backend: &dyn Backend,
    model: &str,
    source_infos: &[smelt_core::sources::SourceInfo],
    source: &str,
    model_target: &str,
    schema: &str,
    file_store: &FileStore,
    state_io_lock: &tokio::sync::Mutex<()>,
) -> Result<
    Option<(
        crate::mutation_probe::MutationVerdict,
        smelt_state::source_mutations::SourceMutationBaseline,
    )>,
> {
    let Some(info) = source_infos.iter().find(|info| {
        let segs = &info.address_segments;
        let bare = match segs.split_first() {
            Some((first, rest)) if first == "sources" => rest.join("."),
            _ => segs.join("."),
        };
        bare == source
    }) else {
        return Ok(None);
    };
    if info.columns.is_empty() {
        return Ok(None);
    }
    let digest_columns: Vec<String> = info.columns.iter().map(|c| c.name.clone()).collect();
    let source_table = info.db_name_for_target(model_target, schema);
    let _io_guard = state_io_lock.lock().await;
    let mutation_baselines = file_store
        .load_source_mutations()
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let (verdict, refreshed) = crate::mutation_probe::gate_upstream_mutation_dispatch(
        backend,
        model,
        source,
        &source_table,
        &digest_columns,
        smelt_backend::maintenance_dialect(backend.dialect()),
        mutation_baselines.get(source),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(Some((verdict, refreshed)))
}

/// Record the refreshed baseline `resolve_upstream_mutation_gate` returned —
/// called only after the licensed technique's write actually succeeded
/// (`docs/specs/incremental_models.md` §"When a mutation cell dispatches":
/// "a failed run cannot suppress the next run's cell"). A `None` gate or a
/// `NoOp` verdict records nothing — the recorded baseline changes only on a
/// genuine dispatch.
pub(crate) async fn record_upstream_mutation_baseline(
    mutation_gate: Option<(
        crate::mutation_probe::MutationVerdict,
        smelt_state::source_mutations::SourceMutationBaseline,
    )>,
    source: &str,
    file_store: &FileStore,
    state_io_lock: &tokio::sync::Mutex<()>,
) {
    let Some((crate::mutation_probe::MutationVerdict::Dispatch, refreshed)) = mutation_gate else {
        return;
    };
    let _io_guard = state_io_lock.lock().await;
    if let Ok(mut mutation_baselines) = file_store.load_source_mutations() {
        mutation_baselines.record(source, refreshed);
        let _ = file_store.save_source_mutations(&mutation_baselines);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_and_dispatch_key_addressed_edge_cell(
    backend: &dyn Backend,
    schema: &str,
    plan_name: &str,
    model_file: &smelt_core::ModelFile,
    clean_sql: &str,
    db_table_name: &str,
    maint_source_facts: &[smelt_logical::maintenance::SourceFacts],
    explicitly_mutable: &HashSet<String>,
    model_edges: &[smelt_logical::maintenance::derive::ModelEdge],
    table_exists_before_run: bool,
    model_by_addr: &HashMap<String, smelt_core::ModelFile>,
    config: &Config,
    request: &ExecuteRequest,
    compiler: &SqlCompiler,
    resolver: &EphemeralResolver,
    run_id: &str,
    reporter: &dyn RunReporter,
    availability: &smelt_logical::maintenance::availability::StateAvailability,
) -> Result<Option<KeyAddressedEdgeDispatch>> {
    if model_edges.is_empty() || !table_exists_before_run {
        return Ok(None);
    }
    let Some(metadata) = model_file.metadata.as_deref() else {
        return Ok(None);
    };
    let Some((edge_name, _cell, key_scope, _upstream_keys, group_key, digest_columns, write)) =
        crate::maintenance_driver::resolve_live_key_addressed_model_edge_cell(
            clean_sql,
            db_table_name,
            metadata,
            maint_source_facts,
            explicitly_mutable,
            model_edges,
            backend.dialect(),
            backend.capabilities().supports_fingerprint_sidecar,
            availability,
        )?
    else {
        return Ok(None);
    };

    let used_per_group_recompute = matches!(
        write,
        crate::maintenance_driver::RepairWrite::TargetedDeleteInsert
    );
    let used_diff_patch = matches!(
        write,
        crate::maintenance_driver::RepairWrite::DiffPatch { .. }
    );
    let retry_policy = RetryPolicy::from_request(request, run_id, plan_name, reporter);
    let upstream_model = model_by_addr.get(&edge_name).ok_or_else(|| {
        anyhow::anyhow!(
            "model '{plan_name}' resolved a live key-addressed model-edge cell on upstream \
             '{edge_name}', but that upstream has no resolved ModelFile — internal \
             inconsistency"
        )
    })?;
    let upstream_target = config.get_target(
        &edge_name,
        upstream_model.metadata.as_deref(),
        &request.target,
    );
    let upstream_schema = &config.targets[&upstream_target].schema;
    let upstream_table = format!("{upstream_schema}.{}", upstream_model.db_name_owned());
    let upstream_source_address = format!("smelt.models.{edge_name}");
    let compiled =
        compiler.compile_with_sql_and_ephemerals(model_file, schema, clean_sql, resolver)?;
    let consumer_address = format!("smelt.models.{}", model_file.canonical_path());
    let result = match crate::maintenance_driver::execute_key_addressed_model_edge_cell(
        backend,
        schema,
        db_table_name,
        &upstream_source_address,
        &upstream_table,
        &group_key,
        &digest_columns,
        &key_scope.keys,
        key_scope.discovery,
        clean_sql,
        &compiled.sql,
        &write,
        &retry_policy,
        &consumer_address,
    )
    .await?
    {
        Some(result) => result,
        None => {
            let row_count = backend
                .get_row_count(schema, db_table_name)
                .await
                .unwrap_or(0);
            smelt_backend::ExecutionResult {
                model_name: db_table_name.to_string(),
                duration: StdDuration::default(),
                row_count,
                preview: None,
            }
        }
    };
    Ok(Some(KeyAddressedEdgeDispatch {
        result,
        used_per_group_recompute,
        used_diff_patch,
        edge_name,
    }))
}

/// Build the per-model `SourceFacts` list and the explicitly-mutable
/// source-name set MP11's live-cell resolvers consume
/// (`resolve_incremental_strategy`, `resolve_live_column_scoped_cell`,
/// `resolve_live_delta_restriction_facts`), for a caller (the dry-run
/// reporting branch) that has not already built them inline. Mirrors the
/// real execution loop's own inline construction exactly (same bare-name
/// convention, same `mutation_profile.kind == Mutable` test) — factored out
/// here so the two call sites cannot silently drift apart.
pub(crate) fn build_maint_source_facts(
    model_file: &smelt_core::ModelFile,
    source_infos: &[smelt_core::sources::SourceInfo],
) -> (
    Vec<smelt_logical::maintenance::SourceFacts>,
    HashSet<String>,
) {
    let mut sources = Vec::new();
    let mut explicitly_mutable = HashSet::new();
    for r in &model_file.refs {
        let segs = r.smelt_ref.to_path();
        let Some(info) = source_infos.iter().find(|s| s.address_segments == segs) else {
            continue;
        };
        let bare = match segs.split_first() {
            Some((first, rest)) if first == "sources" => rest.join("."),
            _ => segs.join("."),
        };
        sources.push(smelt_db::queries::maintenance::source_facts(
            &bare,
            Some(info),
            true,
        ));
        if info
            .mutation_profile
            .as_ref()
            .is_some_and(|m| m.kind == smelt_core::sources::MutationProfile::Mutable)
        {
            explicitly_mutable.insert(bare);
        }
    }
    (sources, explicitly_mutable)
}
