use super::*;
use anyhow::{bail, Result};
use smelt_backend::{maintenance_dialect, Backend, BackendError, ExecutionResult};
use smelt_dialect::SqlDialect;
use smelt_logical::analysis::fingerprint;
use smelt_logical::analysis::fingerprint::Projection as FingerprintProjection;
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::choice::resolve_cell_choice;
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::{KeyDiscovery, PlanCell, SourceFacts, Technique};
use std::collections::HashSet;

/// A live key-addressed model-edge cell as
/// [`resolve_live_key_addressed_model_edge_cell`] returns it: the upstream
/// edge's own name (`== key_scope.from`), the cell itself, its `KeyScope`
/// (the downstream's own key columns to restrict the recompute to), the
/// upstream's own `KeyedUpsert` key columns, the group-grain sidecar's
/// **own** grouping key (`key_scope.keys` for
/// [`KeyDiscovery::DownstreamGrainOverUpstream`], the upstream's own key
/// columns for [`KeyDiscovery::UpstreamKeyed`] — the two coincide only for
/// the upstream-keyed route), the digest column set the sidecar hashes
/// (derived from the downstream's own CLEAN sql — never recomputed against
/// compiled SQL, which carries physical table names rather than `smelt.*`
/// refs the walk-backed fingerprint classifier matches against), and the
/// resolved write leg.
pub type LiveKeyAddressedModelEdgeCell = (
    String,
    PlanCell,
    smelt_logical::maintenance::KeyScope,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    RepairWrite,
);

/// Resolve a `Technique::PerGroupRecompute` cell derived over a
/// **key-addressed model edge** (`docs/specs/incremental_models.md`
/// §"Upstream model edges") — the sibling of
/// [`resolve_live_per_group_recompute_cell`] for a cell whose bounded read is
/// a `KeyScope` (an upstream's own affected key set) rather than a
/// `ScanClamp` over a declared source. The plan is derived exactly once here
/// via [`smelt_db::queries::maintenance::derive_model_maintenance_plan_with_edges`]
/// (maintenance-plan purity, root `CLAUDE.md`) — `model_edges` must be the
/// SAME edge list (with each upstream's own derived `output_shape`) that
/// produced the cell this run will execute.
///
/// Two fail-loud legs run BEFORE any backend call
/// (`docs/outcomes/20260809-output-delta-typing/phases/07-plan.md`):
/// - a target that does not declare `supports_fingerprint_sidecar` — the
///   group-grain sidecar diff this cell's execution needs requires the
///   capability, matching every other sidecar consumer in this module;
/// - a `key_scope.keys` column the upstream relation does not actually
///   carry — checked against the upstream edge's own declared
///   `ModelEdge::unique_key` (the upstream's real output-table column
///   names), never against the downstream's own guess. A mismatch (the
///   downstream renamed the key column it read) is refused by name rather
///   than silently querying a column the upstream table does not have.
#[allow(clippy::too_many_arguments)]
pub fn resolve_live_key_addressed_model_edge_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    model_edges: &[smelt_logical::maintenance::derive::ModelEdge],
    dialect: SqlDialect,
    supports_fingerprint_sidecar: bool,
    availability: &StateAvailability,
) -> Result<Option<LiveKeyAddressedModelEdgeCell>> {
    let Some(result) = crate::maintenance_availability::derive_resolved_with_edges(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        model_edges,
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        availability,
    ) else {
        return Ok(None);
    };

    for cell in &result.plan.cells {
        if cell.technique != Technique::PerGroupRecompute {
            continue;
        }
        let Some(key_scope) = cell.key_scope.clone() else {
            continue;
        };
        let Some(edge) = model_edges.iter().find(|e| e.name == key_scope.from) else {
            bail!(
                "MaintenanceKeyAddressedEdgeMissing: a key-addressed model-edge cell names \
                 upstream '{}' but no matching ModelEdge was supplied — internal \
                 inconsistency between plan derivation and the caller's own edge list",
                key_scope.from
            );
        };
        // The upstream's own proven `KeyedUpsert` key columns — the SAME
        // `edge.output_shape` fact `append_model_edge_cells` reads to admit
        // this cell in the first place (`derive.rs`'s `let Some(OutputDelta::
        // KeyedUpsert { keys }) = &edge.output_shape`), never `edge.
        // unique_key` (a separate, often-undeclared metadata field the walk-
        // derived shape does not depend on).
        let Some(smelt_logical::analysis::output_delta::OutputDelta::KeyedUpsert {
            keys: upstream_keys,
        }) = &edge.output_shape
        else {
            bail!(
                "MaintenanceKeyAddressedEdgeMissing: a key-addressed model-edge cell names \
                 upstream '{}', but its own ModelEdge no longer carries a KeyedUpsert \
                 output_shape — internal inconsistency between plan derivation (which required \
                 this fact to admit the cell) and the caller's own edge list",
                edge.name
            );
        };
        if !supports_fingerprint_sidecar {
            return Err(BackendError::unsupported(
                dialect.name(),
                "key-addressed model-edge affected-key discovery over a KeyedUpsert upstream \
                 (group-grain fingerprint-sidecar diff)",
            )
            .into());
        }
        // The upstream-keyed route's own subset obligation: `key_scope.keys`
        // was resolved by projecting through the upstream's own key
        // columns, so it must literally be a subset of them. The
        // grain-over-upstream route poses no such obligation — its
        // `key_scope` is the downstream's own grain, admitted by
        // `admit_key_addressed_recompute` against the upstream relation's
        // columns directly, not against `upstream_keys`
        // (`docs/specs/incremental_models.md` §"Upstream model edges").
        if key_scope.discovery == KeyDiscovery::UpstreamKeyed
            && !key_scope
                .keys
                .iter()
                .all(|k| upstream_keys.iter().any(|u| u.eq_ignore_ascii_case(k)))
        {
            bail!(
                "MaintenanceKeyScopeColumnMissing: a key-addressed model-edge cell for upstream \
                 '{}' names key column(s) {:?}, but the upstream's own proven KeyedUpsert key \
                 columns are {:?} — a key_scope column absent from the upstream relation is \
                 refused, never widened to every key",
                edge.name,
                key_scope.keys,
                upstream_keys,
            );
        }
        // The group-grain sidecar's own grouping key: the upstream's key
        // columns for the upstream-keyed route (unchanged from before this
        // discovery route existed), or the downstream's own grain
        // (`key_scope.keys`) for the grain-over-upstream route — the sidecar
        // must diff at whichever grain the cell was actually admitted
        // against, never a re-derived one.
        let group_key = match key_scope.discovery {
            KeyDiscovery::UpstreamKeyed => upstream_keys.clone(),
            KeyDiscovery::DownstreamGrainOverUpstream => key_scope.keys.clone(),
        };
        let comparability = model_property_vector(sql, &JoinContext::new())
            .map(|v| v.comparability)
            .unwrap_or_default();
        let chosen = resolve_cell_choice(
            Some(cell),
            &cell.trigger,
            &smelt_logical::maintenance::choice::EffectiveOverride {
                prefer: None,
                technique: None,
            },
            None,
            false,
        )
        .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
        let Some(write) = resolve_repair_write(
            &chosen,
            &key_scope.keys,
            &comparability,
            &cell.row_identity,
            &cell.group,
        )?
        else {
            continue;
        };
        // The digest column set the group-grain sidecar hashes: whatever
        // columns of the upstream this downstream's own (clean) SQL
        // actually reads (`fingerprint::fingerprint_projection`, the same
        // walk-backed leaf classifier `delta_shape_for_source` reuses for
        // the ordinary repair-family sidecar), falling back to the
        // upstream's own key columns alone when the projection cannot be
        // classified — a narrower digest than the ideal (misses a
        // payload-only mutation the downstream's SQL does not itself read),
        // never a widening. Derived here, from `sql` (clean), not at
        // execution time from compiled SQL — a compiled model's `smelt.*`
        // refs are already rewritten to physical table names, which this
        // walk cannot match against.
        let digest_columns: Vec<String> = match fingerprint::fingerprint_projection(sql, &edge.name)
        {
            FingerprintProjection::Columns(cols) => cols.into_iter().collect(),
            FingerprintProjection::FullRow { .. } => upstream_keys.clone(),
        };
        return Ok(Some((
            edge.name.clone(),
            cell.clone(),
            key_scope,
            upstream_keys.clone(),
            group_key,
            digest_columns,
            write,
        )));
    }
    Ok(None)
}

/// The affected-key relation a key-addressed model-edge cell reads
/// (`docs/specs/incremental_models.md` §"Upstream model edges"): the
/// group-grain fingerprint sidecar diff over the upstream's own output
/// table, grouped at `group_key` — the upstream's own `KeyedUpsert` key
/// columns for [`KeyDiscovery::UpstreamKeyed`] (whose changed keys are then
/// forward-projected onto the downstream's own key columns via
/// [`smelt_logical::maintenance::emit::emit_key_addressed_affected_keys_select`]),
/// or the downstream's own grain for
/// [`KeyDiscovery::DownstreamGrainOverUpstream`] (whose diff's own
/// changed-key set already **is** the downstream's affected-key set, so no
/// forward-projection `SELECT` runs — [`repair_keys_literal_select`] wraps
/// the resolved literals directly). A sidecar-capability-gated discovery
/// route — `resolve_live_key_addressed_model_edge_cell` already refused a
/// target lacking `supports_fingerprint_sidecar` before any backend call is
/// reached.
///
/// Returns an empty resolved key list when the sidecar diff discovers no
/// changed keys — the caller reports a no-op rather than executing an
/// empty-but-real write.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_key_addressed_affected_keys(
    backend: &dyn Backend,
    schema: &str,
    upstream_source_address: &str,
    upstream_table: &str,
    upstream_output_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    downstream_keys: &[String],
    discovery: KeyDiscovery,
    model_sql: &str,
    consumer_address: &str,
) -> std::result::Result<(Vec<String>, String), BackendError> {
    let changed_keys = diff_repair_group_sidecar_changed_keys(
        backend,
        schema,
        upstream_source_address,
        upstream_table,
        upstream_output_table,
        group_key,
        digest_columns,
        model_sql,
        consumer_address,
    )
    .await?;
    let dialect = maintenance_dialect(backend.dialect());
    let affected_keys_select = match discovery {
        KeyDiscovery::UpstreamKeyed => {
            smelt_logical::maintenance::emit::emit_key_addressed_affected_keys_select(
                upstream_table,
                group_key,
                downstream_keys,
                &changed_keys,
                dialect,
            )
        }
        KeyDiscovery::DownstreamGrainOverUpstream => {
            repair_keys_literal_select(&changed_keys, dialect)
        }
    };
    Ok((changed_keys, affected_keys_select))
}

/// Execute a live key-addressed model-edge cell
/// ([`resolve_live_key_addressed_model_edge_cell`]): discover the upstream's
/// changed keys via the group-grain fingerprint sidecar diff over its own
/// output table, project them onto the downstream's own key columns
/// ([`resolve_key_addressed_affected_keys`]), stage the downstream's full
/// SQL semi-joined to that key relation
/// ([`repair_candidate_select`]), and write via the resolved
/// [`RepairWrite`] leg — [`execute_per_group_recompute`] for
/// `TargetedDeleteInsert`, [`execute_diff_patch`] for `DiffPatch`. The
/// upstream sidecar refreshes in the SAME backend transaction as this write
/// ([`RepairSidecarRefresh`]), so a failed write never leaves the sidecar
/// advanced past a change it did not actually consume.
///
/// An empty changed-key set is a legitimate no-op: this returns
/// `Ok(None)` rather than executing an empty-but-real write, matching
/// [`emit_key_addressed_affected_keys_select`]'s own well-typed-empty
/// convention (`docs/outcomes/20260809-output-delta-typing/phases/07-plan.md`
/// task 5).
#[allow(clippy::too_many_arguments)]
pub async fn execute_key_addressed_model_edge_cell(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    upstream_source_address: &str,
    upstream_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    downstream_keys: &[String],
    discovery: KeyDiscovery,
    clean_model_sql: &str,
    compiled_model_sql: &str,
    write: &RepairWrite,
    retry: &crate::execute::RetryPolicy<'_>,
    consumer_address: &str,
) -> Result<Option<ExecutionResult>> {
    let full_table = format!("{schema}.{table}");
    let (changed_keys, affected_keys_select) = resolve_key_addressed_affected_keys(
        backend,
        schema,
        upstream_source_address,
        upstream_table,
        &full_table,
        group_key,
        digest_columns,
        downstream_keys,
        discovery,
        clean_model_sql,
        consumer_address,
    )
    .await
    .map_err(|e| {
        anyhow::anyhow!(
            "key-addressed affected-key discovery failed over upstream '{upstream_table}' for \
             '{full_table}': {e}"
        )
    })?;
    if changed_keys.is_empty() {
        return Ok(None);
    }
    let candidate_select =
        repair_candidate_select(compiled_model_sql, downstream_keys, &affected_keys_select);
    let sidecar_refresh = RepairSidecarRefresh {
        schema,
        source_address: upstream_source_address,
        source_table: upstream_table,
        group_key,
        digest_columns,
        model_sql: clean_model_sql,
        consumer_address,
    };
    let result = match write {
        RepairWrite::TargetedDeleteInsert => {
            execute_per_group_recompute(
                backend,
                schema,
                table,
                downstream_keys,
                &affected_keys_select,
                &candidate_select,
                retry,
                Some(&sidecar_refresh),
            )
            .await?
        }
        RepairWrite::DiffPatch {
            compared_columns,
            delete_leg,
        } => {
            let slice_predicate =
                repair_slice_predicate(table, downstream_keys, &affected_keys_select);
            execute_diff_patch(
                backend,
                schema,
                table,
                downstream_keys,
                &candidate_select,
                compared_columns,
                &slice_predicate,
                delete_leg,
                retry,
                Some(&sidecar_refresh),
            )
            .await?
        }
    };
    Ok(Some(result))
}
