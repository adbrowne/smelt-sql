use super::*;

/// Mutation: a post-creation delta in `source` touches exactly the column
/// groups mutation-sensitive to it — the bottom-left column-scoped
/// re-derivation. Partition-local only when the source's partition column is
/// explicitly linked to the output axis (K8's ratified
/// `require: partition_local` refuses the unlinked/unclocked case unless the
/// full scan is declared).
///
/// `source_referential_integrity` is [`mutation_enrichment_closure`]'s own
/// input, threaded straight through — see that function's doc comment for
/// the `None`-vs-attempted-and-`Open` distinction this preserves.
pub(super) fn derive_mutation(
    inputs: &ModelInputs,
    loc: &LocalityInputs<'_>,
    source: &str,
    identity: &RowIdentityVerdict,
    source_referential_integrity: &SourceReferentialIntegrity,
    covered_by_mutation: &BTreeSet<String>,
    plan: &mut MaintenancePlan,
) {
    let trigger = Trigger::UpstreamMutation {
        source: source.to_string(),
    };
    let Some(facts) = inputs.source(source) else {
        plan.refusals.push(Refusal::NoAdmissibleTechnique {
            trigger: format!("{trigger:?}"),
            why: format!("unknown source '{source}'"),
        });
        return;
    };

    // P1 skeleton-source closure (`model_properties.md` §"Skeleton-source
    // closure"; T3 over external sources, `docs/plans/20260715-composed-
    // axes-conditional-maintenance.md` Phase F5): a property of this cell's
    // own enrichment join against `source`, derived once and shared by
    // every column-group cell this source drives — mirroring
    // `append_model_edge_cells`'s `model_edge_enrichment_closure`, the
    // model-edge analogue this generalizes (the "licence union" the phase
    // wires: the SAME `skeleton_source_closure` proof and the SAME
    // `choice::resolve_recompute_restriction` gate now admit an external
    // mutable-snapshot source's enrichment join, not only a model edge's).
    let closure = mutation_enrichment_closure(
        inputs.sql,
        source,
        &inputs.sources,
        source_referential_integrity,
    );

    for group in inputs.column_groups.iter().filter(|g| {
        g.mutation_sensitivity.contains(source) || g.membership_sensitivity.contains(source)
    }) {
        // A `Trigger::NewData { source }` repair cell (`Technique::
        // PerGroupRecompute`, `incremental_models.md` §"The repair family")
        // already recomputes this exact group's bounded, per-key-affected
        // scope every run — including a value change to an already-created
        // row, the SAME thing an `UpstreamMutation` cell would exist to
        // catch. Deriving both would double-write the same group from the
        // same source in the same run (phase 19,
        // `docs/outcomes/20260815-definition-delta-migrate`: widening
        // `UpstreamMutation` derivation to a clocked `MutableSnapshot`
        // source newly makes this collision reachable — the repair family
        // is only ever admitted for exactly the retracting-source shape
        // this trigger also now covers). `triggers` always orders a
        // source's `NewData` before its `UpstreamMutation`
        // (`derive_triggers`), so the repair cell, if any, is already in
        // `plan.cells` by the time this loop runs.
        let already_repaired = plan.cells.iter().any(|c| {
            c.group == group.name()
                && c.technique == Technique::PerGroupRecompute
                && matches!(&c.trigger, Trigger::NewData { source: s } if s == source)
        });
        if already_repaired {
            continue;
        }
        // Membership sensitivity (`docs/specs/incremental_models.md` §"The
        // plan matrix"): a group governed by a row-admission read of
        // `source` must be repaired by a technique that can create and
        // delete rows — the recompute family — even when `source` also
        // happens to be pure value-sensitive for this same group. Only a
        // group whose sensitivity to `source` is *purely* value (never
        // membership) is eligible for the cheaper column-scoped merge.
        let membership_sensitive = group.membership_sensitivity.contains(source);
        let (locality, scans) = match project_source_link(
            inputs.output_partition_col(),
            inputs.keyed_time_axis,
            loc,
            facts,
        ) {
            SourceLink::Clamp(clamp) => (PartitionLocal::Yes, vec![clamp]),
            SourceLink::Unclocked => (
                PartitionLocal::No {
                    source: source.to_string(),
                    why: "unclocked source: a change's footprint projects onto no bounded \
                          partition interval of the output"
                        .to_string(),
                },
                vec![],
            ),
            SourceLink::Unlinked { why } => (
                PartitionLocal::No {
                    source: source.to_string(),
                    why,
                },
                vec![],
            ),
        };
        if matches!(locality, PartitionLocal::No { .. }) && !facts.allow_full_scan {
            plan.refusals.push(Refusal::ScanUnbounded {
                source: source.to_string(),
                why: format!(
                    "maintenance of {} driven by '{source}' scatters across all output \
                     partitions; declare allow_full_scan to accept the full-table write",
                    group.name()
                ),
            });
            continue;
        }
        // Group-merge-provenance guard (`incremental_models.md` §"The plan
        // matrix", decided in success criterion 18's "Group-merge-provenance
        // policy" open question): a group whose sensitivity spans TWO OR
        // MORE mutation-capable inputs is repaired by region recompute,
        // never a column-scoped merge — the conservative, always-correct
        // default every other mutation-sensitivity rule here already takes.
        // "Mutation-capable" means the source actually gets its own
        // `UpstreamMutation` trigger (`covered_by_mutation`, read straight
        // off `triggers` — the SAME predicate `derive_triggers` already
        // computed, not a second guess): an append-only source with no
        // value-sensitivity of its own, or one this model doesn't derive a
        // mutation cell for, does not count toward the merge, matching
        // `membership_sensitive`'s existing scoping to genuinely
        // mutation-driven groups.
        let mutation_capable_inputs = group
            .mutation_sensitivity
            .union(&group.membership_sensitivity)
            .filter(|s| covered_by_mutation.contains(*s))
            .count();
        // A `ChangeFeed` source's cell is clamped to full-input
        // re-derivation, the same forcing shape the merged-group guard
        // above uses: no column-scoped merge or fold realisation exists
        // for a posture whose delta shape is never read
        // (`incremental_models.md` §Known Divergences).
        let (corner, technique) = if facts.mutation == MutationProfile::ChangeFeed
            || membership_sensitive
            || mutation_capable_inputs >= 2
        {
            (Corner::RecomputeRegion, Technique::DeleteInsert)
        } else {
            (Corner::ColumnMerge, Technique::ColumnScopedMerge)
        };
        plan.cells.push(PlanCell {
            group: group.name(),
            trigger: trigger.clone(),
            corner,
            technique,
            partition_local: locality,
            scans,
            ledger_catch_up: false,
            row_identity: identity.clone(),
            skeleton_source_closure: closure.clone(),
            fingerprint_projections: BTreeMap::new(),
            key_scope: None,
            state_downgrade: None,
        });
    }
}
