use super::*;
use anyhow::Result;
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::choice::{
    effective_override, resolve_region_write_variant, ChoiceRefusal, RegionWrite,
};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::{RowIdentity, SkeletonSourceClosure, SourceFacts, Trigger};
use std::collections::HashSet;

/// The facts [`build_delete_insert_group_dispatched`]/
/// [`execute_delete_insert_with_delta_restriction`] need to attempt T3 delta
/// restriction for a model-edge-sourced creation cell, resolved by
/// [`resolve_live_delta_restriction_facts`].
#[derive(Debug, Clone)]
pub struct DeltaRestrictionFacts {
    /// The driving model edge's bare address (`Trigger::NewData`'s `source`
    /// name) — the upstream whose observed-delta table is read.
    pub upstream_model: String,
    /// The model's own region row identity, when it resolves to exactly one
    /// column (`RowIdentity::Key(_)` with one element) — this phase's
    /// semi-join restriction is single-column only. `None` for a composite
    /// key or `RowIdentity::WholeRow`, in which case the caller must fall
    /// back to the ordinary widened scan (matching an absent P1 closure).
    pub restrict_column: Option<String>,
    /// The cell's P1 skeleton-source-closure verdict, carried through
    /// unchanged for [`resolve_recompute_restriction`] to consult.
    pub skeleton_source_closure: Option<SkeletonSourceClosure>,
    /// The region family's own change-suppressed conditional-write verdict
    /// ([`RegionWrite`]), resolved from the SAME `Trigger::NewData` cell
    /// this struct already reads — the T3 delta-restriction arm wins when
    /// both are admitted ([`build_delete_insert_group_dispatched`]'s own
    /// match order), so this is consulted only as the fallback dimension.
    pub region_write: RegionWrite,
}

/// Resolve [`DeltaRestrictionFacts`] for a model driven (at least in part)
/// by an upstream **maintained-model** edge (`model_edges`, built by the
/// caller mirroring `crate::propagation::derive_clamp_and_locality`'s own
/// edge extraction — never re-derived here). Routes through the SAME
/// edge-aware derivation `smelt explain`/the propagation graph already
/// consume (`derive_model_maintenance_plan_with_edges` →
/// `append_model_edge_cells`) rather than re-implementing admission
/// (`CLAUDE.md` §"Maintenance-plan purity").
///
/// `model_edges.first()` is this call's driving edge: `append_model_edge_
/// cells` derives ONE shared P1 closure verdict for every edge of a model
/// (see that function's own doc comment — the verdict is a property of the
/// model's own query shape, not of which edge triggered the recompute), so
/// picking any one edge's cell yields the same closure either way; the
/// first edge is simply the one whose observed-delta table the caller then
/// reads. A model with more than one maintained-model upstream restricts
/// only against this first edge's delta in this phase — a later phase may
/// widen this to try every edge in turn.
///
/// Returns `None` when `model_edges` is empty, the plan derives no creation
/// cell for the driving edge (e.g. `Refusal::ReachNotDerivable`), or
/// `metadata`'s resolved grain has no partition axis for a model edge to
/// clamp to — the caller's safe default in every `None` case is the
/// ordinary widened scan.
///
/// The [`RegionWrite`] variant is resolved from the SAME cell via
/// [`resolve_region_write_variant`] — composed from the shared P2/P3 proof
/// (`choice::resolve_write_suppression`/`resolve_write_variant`), never a
/// fresh derivation. A `technique: suppress`/`unconditional` pin on this
/// cell's own trigger address is consulted the same way every other
/// dispatch resolver in this module consults it ([`effective_override`]); an
/// inadmissible hard pin surfaces as a real `Err`, never a silent fallback
/// to the unconditional widened scan.
pub fn resolve_live_delta_restriction_facts(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    model_edges: &[smelt_logical::maintenance::derive::ModelEdge],
    availability: &StateAvailability,
) -> Result<Option<DeltaRestrictionFacts>, ChoiceRefusal> {
    let Some(driving_edge) = model_edges.first() else {
        return Ok(None);
    };
    let Some(result) = crate::maintenance_availability::derive_resolved_with_edges(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        model_edges,
        // See `resolve_incremental_strategy`'s analogous call: not (yet)
        // plumbed with a driving-source granularity or declared
        // `key_recurrence` bounds at this call site — this resolver only
        // reads the model-edge creation cell's closure/row-identity facts,
        // which key temporal locality's routes do not gate.
        None,
        &[],
        // This resolver only reads the model-edge `NewData` creation cell —
        // a `ColumnAdded` trigger never affects it, so no deployed-schema
        // snapshot is needed here.
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        availability,
        &[],
    ) else {
        return Ok(None);
    };
    let Some(cell) = result.plan.cell_for(&Trigger::NewData {
        source: driving_edge.name.clone(),
    }) else {
        return Ok(None);
    };
    let restrict_column = match &cell.row_identity.identity {
        RowIdentity::Key(cols) if cols.len() == 1 => Some(cols[0].clone()),
        _ => None,
    };
    // A model-edge creation cell's `group` is the literal whole-row
    // placeholder (`"{*}"`, `append_model_edge_cells`'s own `PlanCell`
    // construction) — there is no NAMED column group in `result.column_
    // groups` to look it up by, unlike a mutation-sensitive `UpstreamMutation`
    // cell's own derived group. The region family's write covers the whole
    // output row, so its own compared-column set is the model's entire
    // output projection MINUS the proven row-identity's own key columns
    // (comparing a join key via `IS DISTINCT FROM` against itself is
    // vacuous — the diff join already pins it equal for every matched row)
    // — the same `PropertyVector` this call's own comparability read already
    // derives, read once and reused for both.
    let vector = model_property_vector(sql, &JoinContext::new());
    let key_columns: &[String] = match &cell.row_identity.identity {
        RowIdentity::Key(cols) => cols,
        RowIdentity::WholeRow => &[],
    };
    let group_columns: Vec<String> = vector
        .as_ref()
        .map(|v| {
            v.columns
                .iter()
                .filter(|c| !key_columns.iter().any(|k| k.eq_ignore_ascii_case(c)))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let comparability = vector.map(|v| v.comparability).unwrap_or_default();
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    let overrides = effective_override(
        metadata
            .maintenance
            .as_ref()
            .and_then(|m| m.defaults.as_ref()),
        cells_cfg,
        &driving_edge.name,
        &group_columns,
    );
    let region_write = resolve_region_write_variant(
        &group_columns,
        &comparability,
        &cell.row_identity,
        &cell.trigger,
        cell.ledger_catch_up,
        &overrides,
    )?;
    Ok(Some(DeltaRestrictionFacts {
        upstream_model: driving_edge.name.clone(),
        restrict_column,
        skeleton_source_closure: cell.skeleton_source_closure.clone(),
        region_write,
    }))
}

/// The facts [`execute_delete_insert_with_delta_restriction`]'s
/// `RestrictionDeltaSource::ExternalSidecar` arm needs to attempt T3 delta
/// restriction for a region-family cell driven by an external
/// `mutable_snapshot` source with no native change feed, resolved by
/// [`resolve_live_external_delta_restriction_facts`].
#[derive(Debug, Clone)]
pub struct ExternalDeltaRestrictionFacts {
    /// The external source's bare address (`Trigger::UpstreamMutation`'s
    /// `source` name) — the sidecar partition this cell diffs against.
    pub source_name: String,
    /// The model's own region row identity, when it resolves to exactly one
    /// column — mirrors [`DeltaRestrictionFacts::restrict_column`].
    pub restrict_column: Option<String>,
    /// The cell's P1 skeleton-source-closure verdict.
    pub skeleton_source_closure: Option<SkeletonSourceClosure>,
    /// The P4 fingerprint projection the sidecar digests over this source.
    pub projection: FingerprintProjection,
    /// The region family's own change-suppressed conditional-write verdict,
    /// resolved from the SAME `Trigger::UpstreamMutation` cell this struct
    /// already reads.
    pub region_write: RegionWrite,
}

/// Resolve [`ExternalDeltaRestrictionFacts`] for a model driven by an
/// explicitly-mutable **external** source with no maintained-model upstream
/// (`model_edges` empty is the caller's own gate —
/// `docs/outcomes/20260815-definition-delta-migrate/phases/27e-plan.md`).
/// Routes through the SAME edge-aware derivation
/// [`resolve_live_delta_restriction_facts`] uses
/// (`derive_model_maintenance_plan_with_edges` → the `Trigger::
/// UpstreamMutation` cell this source drives), never re-deriving admission
/// (`CLAUDE.md` §"Maintenance-plan purity").
///
/// `explicitly_mutable`'s driving source is picked deterministically
/// (lexicographically smallest — `HashSet` iteration order is unspecified),
/// mirroring `resolve_live_delta_restriction_facts`'s `model_edges.first()`
/// "restrict only against one driving delta in this phase" scoping.
///
/// Returns `None` — logging a `tracing::debug!` `why` — when
/// `supports_fingerprint_sidecar` is `false` for this backend, no
/// explicitly-mutable source exists, the plan derives no `UpstreamMutation`
/// cell for it, the closure is `Open`/absent, the source's declared
/// `unique_key` is composite/undeclared
/// ([`enrichment_restrict_column`] returns `None`), or no P4 fingerprint
/// projection resolved for it — every `None` case is the caller's safe
/// default of falling back to the ordinary widened scan.
#[allow(clippy::too_many_arguments)]
pub fn resolve_live_external_delta_restriction_facts(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    source_referential_integrity: &SourceReferentialIntegrity,
    supports_fingerprint_sidecar: bool,
    availability: &StateAvailability,
) -> Result<Option<ExternalDeltaRestrictionFacts>, ChoiceRefusal> {
    if !supports_fingerprint_sidecar {
        tracing::debug!(
            table,
            "backend does not declare supports_fingerprint_sidecar — external delta \
             restriction stays out of reach, falling back to the widened scan"
        );
        return Ok(None);
    }
    let mut candidates: Vec<&String> = explicitly_mutable.iter().collect();
    candidates.sort();
    let Some(source_name) = candidates.into_iter().next() else {
        return Ok(None);
    };
    let Some(source_facts) = sources.iter().find(|s| &s.name == source_name) else {
        return Ok(None);
    };
    let Some(result) = crate::maintenance_availability::derive_resolved_with_edges(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        &[],
        None,
        &[],
        &[],
        source_referential_integrity,
        None,
        None,
        availability,
        &[],
    ) else {
        return Ok(None);
    };
    let trigger = Trigger::UpstreamMutation {
        source: source_name.clone(),
    };
    let Some(cell) = result.plan.cell_for(&trigger) else {
        tracing::debug!(
            table,
            source = %source_name,
            "no UpstreamMutation cell admitted for this source — external delta restriction \
             falls back to the widened scan"
        );
        return Ok(None);
    };
    if !cell
        .skeleton_source_closure
        .as_ref()
        .is_some_and(|c| c.is_closed())
    {
        tracing::debug!(
            table,
            source = %source_name,
            "skeleton-source closure is Open/absent — external delta restriction falls back \
             to the widened scan"
        );
        return Ok(None);
    }
    let Some(restrict_column) = enrichment_restrict_column(&source_facts.unique_key) else {
        tracing::debug!(
            table,
            source = %source_name,
            "source's declared unique_key is composite or undeclared — external delta \
             restriction falls back to the widened scan"
        );
        return Ok(None);
    };
    let Some(projection) = cell.fingerprint_projections.get(source_name).cloned() else {
        tracing::debug!(
            table,
            source = %source_name,
            "no P4 fingerprint projection resolved for this source — external delta \
             restriction falls back to the widened scan"
        );
        return Ok(None);
    };
    let group_columns: Vec<String> = result
        .column_groups
        .iter()
        .find(|g| g.name() == cell.group)
        .map(|g| g.columns.clone())
        .unwrap_or_default();
    let comparability = model_property_vector(sql, &JoinContext::new())
        .map(|v| v.comparability)
        .unwrap_or_default();
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    let overrides = effective_override(
        metadata
            .maintenance
            .as_ref()
            .and_then(|m| m.defaults.as_ref()),
        cells_cfg,
        source_name,
        &group_columns,
    );
    let region_write = resolve_region_write_variant(
        &group_columns,
        &comparability,
        &cell.row_identity,
        &cell.trigger,
        cell.ledger_catch_up,
        &overrides,
    )?;
    Ok(Some(ExternalDeltaRestrictionFacts {
        source_name: source_name.clone(),
        restrict_column: Some(restrict_column.to_string()),
        skeleton_source_closure: cell.skeleton_source_closure.clone(),
        projection,
        region_write,
    }))
}
