use super::*;

/// Backfill: the universal ground-truth reset — recompute the region from
/// replayable input, unconditionally correct (`01-framework.md` §3).
pub(super) fn derive_backfill(
    inputs: &ModelInputs,
    loc: &LocalityInputs<'_>,
    identity: &RowIdentityVerdict,
    plan: &mut MaintenancePlan,
) {
    let (partition_local, scans) = read_locality(inputs, loc);
    plan.cells.push(PlanCell {
        group: "{*}".to_string(),
        trigger: Trigger::Backfill,
        corner: Corner::RecomputeRegion,
        technique: Technique::DeleteInsert,
        partition_local,
        scans,
        ledger_catch_up: false,
        row_identity: identity.clone(),
        skeleton_source_closure: None,
        fingerprint_projections: BTreeMap::new(),
        key_scope: None,
        state_downgrade: None,
    });
}

/// Partition-locality of a whole-row recompute's *reads*, plus the derived
/// scan clamps for the sources that are linked. The first unlinked or
/// unclocked source decides the `No` verdict (backfill stays admitted — a
/// recompute is the universal ground-truth reset — but the full read is
/// named, never silent).
pub(super) fn read_locality(
    inputs: &ModelInputs,
    loc: &LocalityInputs<'_>,
) -> (PartitionLocal, Vec<ScanClamp>) {
    // Keyed grain: a whole-table rebuild over a key-addressed output — there
    // is no output partition axis to be local to, so the locality question
    // is not posed and the sentinel `PartitionLocal::Yes` records the
    // vacuous verdict (policy, not a proof outcome — the locality proof is
    // only ever consulted against a partition-addressed output;
    // `model_properties.md` §Known Divergences keeps this keyed residue).
    if inputs.output_partition_col().is_none() {
        return (PartitionLocal::Yes, vec![]);
    }
    let mut scans = Vec::new();
    let mut verdict = PartitionLocal::Yes;
    for s in &inputs.sources {
        match project_source_link(
            inputs.output_partition_col(),
            inputs.keyed_time_axis,
            loc,
            s,
        ) {
            SourceLink::Clamp(clamp) => scans.push(clamp),
            SourceLink::Unclocked => {
                if matches!(verdict, PartitionLocal::Yes) {
                    verdict = PartitionLocal::No {
                        source: s.name.clone(),
                        why: "unclocked source is read in full on every recompute".to_string(),
                    };
                }
            }
            SourceLink::Unlinked { why } => {
                if matches!(verdict, PartitionLocal::Yes) {
                    verdict = PartitionLocal::No {
                        source: s.name.clone(),
                        why,
                    };
                }
            }
        }
    }
    (verdict, scans)
}

/// Convenience used by tests: the set of column names across `groups`.
pub fn group_columns(groups: &[ColumnGroup]) -> BTreeSet<String> {
    groups
        .iter()
        .flat_map(|g| g.columns.iter().cloned())
        .collect()
}
