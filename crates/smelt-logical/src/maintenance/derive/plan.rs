use super::*;

/// Derive the plan cells (and refusals) for `triggers` against `inputs`.
///
/// Every `UpstreamMutation` cell's `skeleton_source_closure` is `None` —
/// this entry point never attempts the P1 proof for an external source's
/// enrichment join (byte-identical to this function's pre-Phase-F5
/// behaviour). Use [`derive_maintenance_plan_with_referential_integrity`]
/// to opt an external source's declared `referential_integrity` world-fact
/// into the same proof [`append_model_edge_cells`] already runs for model
/// edges.
pub fn derive_maintenance_plan(inputs: &ModelInputs, triggers: &[Trigger]) -> MaintenancePlan {
    derive_maintenance_plan_impl(inputs, triggers, &SourceReferentialIntegrity::new())
}

/// [`derive_maintenance_plan`], additionally threading `source_referential_
/// integrity` world-facts (`docs/specs/sources.md` §"Referential
/// integrity") into every `UpstreamMutation` cell's P1 skeleton-source-
/// closure proof (T3 over external sources, `docs/plans/
/// 20260715-composed-axes-conditional-maintenance.md` Phase F5) — the
/// licence union with [`append_model_edge_cells`]'s already-landed model-
/// edge proof: the SAME [`crate::analysis::skeleton_closure::
/// skeleton_source_closure`] function, the SAME [`super::choice::
/// resolve_recompute_restriction`] gate downstream, no new mechanism.
///
/// A source absent from `source_referential_integrity` behaves exactly as
/// under [`derive_maintenance_plan`] (`skeleton_source_closure: None`) —
/// this function only *adds* closure attempts for the sources the caller
/// names, never removes or alters anything else `derive_maintenance_plan`
/// would have derived.
pub fn derive_maintenance_plan_with_referential_integrity(
    inputs: &ModelInputs,
    triggers: &[Trigger],
    source_referential_integrity: &SourceReferentialIntegrity,
) -> MaintenancePlan {
    derive_maintenance_plan_impl(inputs, triggers, source_referential_integrity)
}

fn derive_maintenance_plan_impl(
    inputs: &ModelInputs,
    triggers: &[Trigger],
    source_referential_integrity: &SourceReferentialIntegrity,
) -> MaintenancePlan {
    let mut plan = MaintenancePlan::default();
    if let Some(reason) = skeleton_clause_changed(inputs) {
        plan.refusals
            .push(Refusal::SkeletonClauseChanged { reason });
    }
    if let Some((from, to)) = partition_column_changed(inputs) {
        plan.refusals
            .push(Refusal::PartitionColumnChanged { from, to });
    }
    let bounds = derive_model_bounds(inputs.sql, &inputs.bound_context());
    // The write-scope dual of `bounds` (`model_properties.md` §"Footprint
    // reflection / bounded write footprint"), derived once per model and
    // consulted at every clamp-construction site via `project_source_link`. A
    // key-grain output has no partition axis to spread a write across, so
    // the footprint question is not posed (empty map — the keyed residue
    // policy in `project_source_link` links instead).
    let footprints = match inputs.output_partition_col().or(inputs.keyed_time_axis) {
        Some(axis) => reflect_footprint(inputs.sql, &inputs.bound_context(), Some(axis)),
        None => HashMap::new(),
    };
    // The cross-axis predicate evidence (`model_properties.md`
    // §"Partition-locality projection") — like the footprint, posed only
    // against a partition-addressed output (a keyed output has no axis to
    // link a source's partition column to; `project_source_link` keeps the
    // documented keyed residue policy instead).
    let links = match inputs.output_partition_col() {
        Some(axis) => derive_cross_axis_links(inputs.sql, &inputs.bound_context(), axis),
        None => HashMap::new(),
    };
    let loc = LocalityInputs {
        bounds: &bounds,
        footprints: &footprints,
        links: &links,
    };
    let identity = row_identity(inputs.declared_unique_key(), inputs.sql);
    // The set of sources this model's own trigger list covers with an
    // `UpstreamMutation` cell — i.e. every source name for which `triggers`
    // (built by the caller, e.g. `smelt-db::queries::maintenance::
    // derive_model_maintenance_plan`, under the unclocked +
    // `explicitly_mutable` predicate ≈ that function's L397-404) already
    // contains a `Trigger::UpstreamMutation`. Read straight off `triggers`
    // rather than re-deriving the predicate here: `triggers` IS that
    // predicate's own output, so this is one source of truth, not a second
    // copy that could drift. Consulted by `derive_new_data`'s key-grain
    // branch (`incremental_shapes.md` §"The key grain (`grain: key`)") to
    // waive the append-only obligation for a source maintained by a covered
    // enrichment cell instead of folded.
    let covered_by_mutation: BTreeSet<String> = triggers
        .iter()
        .filter_map(|t| match t {
            Trigger::UpstreamMutation { source } => Some(source.clone()),
            _ => None,
        })
        .collect();

    for trigger in triggers {
        match trigger {
            Trigger::NewData { source } => derive_new_data(
                inputs,
                &loc,
                source,
                &identity,
                &covered_by_mutation,
                &mut plan,
            ),
            Trigger::UpstreamMutation { source } => derive_mutation(
                inputs,
                &loc,
                source,
                &identity,
                source_referential_integrity,
                &covered_by_mutation,
                &mut plan,
            ),
            Trigger::ColumnAdded { columns } => {
                derive_column_added(inputs, &loc, columns, &identity, &mut plan)
            }
            Trigger::Backfill => derive_backfill(inputs, &loc, &identity, &mut plan),
        }
    }

    // P4 fingerprint projection (`model_properties.md` §"Fingerprint
    // projection"): a property of the model's own SQL against each
    // declared source, not of any one trigger/technique — derived once and
    // shared across every cell this model produced, mirroring how
    // `identity` above is one row-identity verdict shared by every cell.
    let projections = model_fingerprint_projections(inputs);
    if !projections.is_empty() {
        for cell in &mut plan.cells {
            cell.fingerprint_projections = projections.clone();
        }
    }

    plan
}

/// Derive the P4 fingerprint-projection verdict (`model_properties.md`
/// §"Fingerprint projection") of `inputs.sql` against every one of
/// `inputs.sources` — the column set a row-content fingerprint sidecar
/// would digest for each. Pure data; no sidecar/digest machinery here
/// (that is F3's scope).
fn model_fingerprint_projections(inputs: &ModelInputs) -> BTreeMap<String, FingerprintProjection> {
    inputs
        .sources
        .iter()
        .map(|s| (s.name.clone(), fingerprint_projection(inputs.sql, &s.name)))
        .collect()
}
