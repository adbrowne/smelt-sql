use super::*;

/// Creation: new rows in the driving source. Partition grain recomputes the
/// new region (today's mechanism — for a pure append the RMW corner
/// degenerates to the same insert); key grain folds the delta into stored
/// key state, admitted only for a faithful additive combiner over an
/// append-only source (`01-framework.md` §4).
pub(super) fn derive_new_data(
    inputs: &ModelInputs,
    loc: &LocalityInputs<'_>,
    source: &str,
    identity: &RowIdentityVerdict,
    covered_by_mutation: &BTreeSet<String>,
    plan: &mut MaintenancePlan,
) {
    let trigger = Trigger::NewData {
        source: source.to_string(),
    };
    match &inputs.output.grain {
        Grain::Partition { .. } => {
            let (partition_local, scans) = read_locality(inputs, loc);
            plan.cells.push(PlanCell {
                group: "{*}".to_string(),
                trigger,
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
        Grain::Key { unique_key } => {
            let Some(facts) = inputs.source(source) else {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!("unknown source '{source}'"),
                });
                return;
            };
            let Some(fold) = &inputs.fold else {
                // No recognised fold-family column (`FoldSpec` only ever
                // carries additive/extremal/order-monotone combiners,
                // `smelt-db::queries::maintenance::derive_fold_spec`) over
                // an UNCLOCKED source, WITH a proven non-empty key (a real
                // `GROUP BY`, not a degenerate/malformed declaration), is
                // exactly the plain-overwrite/snapshot-reconcile shape
                // (`docs/specs/incremental_shapes.md` §"The two run
                // shapes"; `docs/plans/20260809-keyed-frontier.md` Phase
                // 3) — a model whose only non-key columns are
                // `ANY_VALUE(...)` (or none at all). That shape is not
                // driven by a `NewData` fold cell at all (the
                // snapshot-reconcile executor re-scans the whole source
                // every run, `smelt-runtime::cumulative::execute_
                // snapshot_reconcile`), so this trigger needs neither a
                // cell nor a refusal — silently skip, mirroring the
                // enrich-only waiver above `derive_new_data`'s obligation-2
                // narrowing. An empty `unique_key` means the model never
                // proved a real keyed grain in the first place (e.g. no
                // `GROUP BY` at all) — a genuinely different, malformed
                // shape that keeps the pre-existing refusal regardless of
                // clock; a CLOCKED source with no fold columns (but a real
                // key) is likewise the pre-existing degenerate-refusal
                // case, unaffected by this narrowing.
                if facts.partition_col.is_none() && !unique_key.is_empty() {
                    return;
                }
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: "keyed grain with no fold specification".to_string(),
                });
                return;
            };

            // Per-cell admission obligation 2 (`incremental_models.md`
            // §"Per-cell admission"): the faithful fold's two INDEPENDENT
            // conditions — source posture (does the delta stream partition
            // the input, i.e. is it retraction-free) and combiner algebra
            // (can a retracted contribution be undone) — either failing
            // alone refuses the fold family for this cell
            // (`model_properties.md` §"Faithful-fold conditions"). Obligation
            // 3 (combiner algebra class) is checked independently of source
            // posture: a holistic/unrecognised combiner refuses regardless of
            // how clean the source is, and leaves only the recompute family
            // admissible for this cell (no fold cell is synthesized in v0 —
            // `derive_backfill`/a declared `full` refresh is that family's
            // representative today; wiring the fallback as an alternate
            // technique inside the same cell is deferred, since v0 admits at
            // most one technique per cell). Checked per column below —
            // obligation 3 is independent per combiner, so a mixed fold
            // (e.g. `SUM` alongside `MIN`/`MAX`) refuses as a whole the
            // moment any one column's combiner fails it.

            // Obligation 2, source-posture half: `input_delta_discovery` is
            // the SC-2 tripwire's (`docs/research/property-discovery/
            // ledger.md`) production consumer. A clocked `Mutable` source's
            // `WindowForward` discovery only proves *how new rows are found*
            // — it has no branch for an in-place update to an
            // already-processed partition, so it can never by itself widen a
            // source to "retraction-free". The declared `MutationProfile`
            // remains the sole source of that fact (never derived from
            // discovery kind alone) — inside the proof, `discovery` refines
            // only the failure *reason*, never the verdict.
            let discovery = input_delta_discovery(source_shape(facts));

            // Narrowing (`incremental_shapes.md` §"The key grain
            // (`grain: key`)"), applied BEFORE the faithful-fold proof is
            // consulted — this is derive-layer waiver POLICY, not part of
            // the proof: the append-only obligation binds a
            // FOLD-CONTRIBUTING source, not every source the model
            // references. This `NewData` trigger's obligation is waived
            // iff (i) `source` is covered by an `UpstreamMutation` cell
            // for this model — its post-creation mutations are
            // maintained by that cell, not silently dropped — AND (ii)
            // `source_contributes_to_fold` proves `source` is never an
            // argument to the fold's own aggregates. Both conditions are
            // required: coverage alone would let an un-retractable
            // folded contribution through (the classifier's
            // conservatism is the safety net there); non-contribution
            // alone would still fold an un-retractable delta with
            // nothing else maintaining it. A source that is both
            // fold-contributing and mutable stays refused below — the
            // folded contribution genuinely is un-retractable. When
            // waived, this `NewData{source}` trigger needs no technique
            // at all (no cell, no refusal): `source`'s deltas do not
            // feed the fold, and its post-creation mutations are already
            // this model's `UpstreamMutation{source}` cell's job, not
            // this one's.
            if facts.mutation != MutationProfile::AppendOnly
                && covered_by_mutation.contains(source)
                && !source_contributes_to_fold(inputs.sql, source)
            {
                return;
            }

            // Obligations 2 and 3 are the two faithful-fold conditions
            // (`model_properties.md` §"Faithful-fold conditions"), derived
            // by the pure `faithful_fold` proof; this function only maps a
            // failing verdict onto the existing refusal text. Condition (1)
            // is combiner-independent, so one representative verdict decides
            // it for the whole fold (`Count` stands in when the fold has no
            // add columns — the posture obligation still binds).
            let posture = match facts.mutation {
                MutationProfile::AppendOnly => DeltaMutationProfile::AppendOnly,
                MutationProfile::MutableSnapshot => DeltaMutationProfile::Mutable,
                MutationProfile::ChangeFeed => DeltaMutationProfile::ChangeFeed,
            };
            let representative = fold
                .add_columns
                .first()
                .map(|(_, c)| *c)
                .unwrap_or(SqlFunction::Count);
            if let FaithfulFold::Fails {
                partitioned_input: ConditionVerdict::Fails { reason },
                ..
            } = faithful_fold(representative, false, &posture, discovery)
            {
                // Repair narrowing (`incremental_models.md` §"The repair
                // family"): the faithful-fold's source-posture obligation is
                // exactly the retraction case the repair family exists for —
                // before refusing outright, attempt the per-group recompute
                // technique. Repair only ever converts a refusal into a
                // cell; it never replaces an already-admitted fold, so this
                // branch (already established: the posture obligation
                // failed) is the only site it can fire from. When repair
                // admission also fails, the pre-existing
                // `NoAdmissibleTechnique` refusal is still pushed, and the
                // repair refusal is pushed alongside it naming the failing
                // obligation — additive, not a replacement.
                //
                // A `ChangeFeed` source has no fingerprint-sidecar to diff
                // (`repair::discovery_posture` has no `SidecarDiff` arm for
                // it — the feed's delta shape isn't consumed yet,
                // `incremental_models.md` §Known Divergences), so the repair
                // family is refused here, loud and named, rather than
                // attempted against a discovery posture that doesn't exist.
                if facts.mutation == MutationProfile::ChangeFeed {
                    plan.refusals.push(Refusal::NoAdmissibleTechnique {
                        trigger: format!("{trigger:?}"),
                        why: format!(
                            "fold over '{source}' fails the faithful-fold source-posture \
                             condition: {reason}, and the repair family has no fingerprint-\
                             sidecar discovery for a change_feed source",
                        ),
                    });
                    return;
                }
                let delta = repair::delta_shape_for_source(inputs.sql, facts);
                // The SAME declared-`unique_key` facts `append_model_edge_cells`
                // folds into its own `join_ctx` (`source_facts_join_context`),
                // built here for the repair route's affected-key discovery
                // rather than an always-empty context (`docs/outcomes/
                // 20260904-walk-migration-residue/outcome.md` phase 5).
                let repair_join_ctx = source_facts_join_context(inputs.sql, &inputs.sources);
                match repair::admit_per_group_recompute(
                    inputs.sql,
                    unique_key,
                    facts,
                    inputs.output_partition_col(),
                    inputs.keyed_time_axis,
                    loc,
                    &delta,
                    &repair_join_ctx,
                ) {
                    Ok(admitted) => {
                        // Alphabetically sorted, matching
                        // `ColumnGroup::name()`'s own convention
                        // (`grouping::derive_column_groups` buckets columns
                        // via a `BTreeMap<String, _>` keyed by column name,
                        // never SQL declaration order) — `matching_write_pin`
                        // compares this string against a `ColumnGroup`'s own
                        // `name()` by exact equality, so a repair cell whose
                        // presented columns aren't already alphabetical in
                        // the SQL (e.g. `OrderMonotone`'s `(max_by_val,
                        // max_by_ord)`) must still agree with it.
                        let mut add_column_names: Vec<String> = fold
                            .add_columns
                            .iter()
                            .map(|(name, _)| name.clone())
                            .collect();
                        add_column_names.sort();
                        plan.cells.push(repair::derive_repair_cell(
                            &admitted,
                            trigger,
                            format!("{{{}}}", add_column_names.join(", ")),
                        ));
                    }
                    Err(refusal) => {
                        plan.refusals.push(Refusal::NoAdmissibleTechnique {
                            trigger: format!("{trigger:?}"),
                            why: format!(
                                "fold over '{source}' fails the faithful-fold source-posture \
                                 condition: {reason} (whether or not any of the fold's combiners \
                                 ({:?}) are themselves monoids)",
                                fold.add_columns.iter().map(|(_, c)| *c).collect::<Vec<_>>()
                            ),
                        });
                        let (repair_why, repair_refusal) = match refusal {
                            repair::RepairRefusal::KeysNotDiscoverable { source, why } => (
                                why.clone(),
                                Refusal::RepairKeysNotDiscoverable { source, why },
                            ),
                            repair::RepairRefusal::SliceUnbounded { source, why } => {
                                (why.clone(), Refusal::RepairSliceUnbounded { source, why })
                            }
                        };
                        plan.refusals.push(repair_refusal);

                        // `KeyedRetractableContribution`
                        // (`incremental_shapes.md` §"Enrichment joins"): the
                        // repair family just failed to cover this source's
                        // retraction (the arm we're already in) — test
                        // whether the failure is specifically a retractable
                        // ENRICHMENT-JOIN contribution, composing the
                        // already-derived join cardinality with each fed
                        // fold column's combiner algebra
                        // (`join_shape::join_contribution_monotone`), mirroring
                        // `dimension_join_contribution`'s own ctx-building
                        // (keyed on the join's resolved ALIAS, never the bare
                        // source name — a join condition qualifies columns
                        // by alias). Never fires on join spelling alone:
                        // `join_alias_for_source` returning `None` (no join
                        // against `source` at all) pushes nothing, matching
                        // `dimension_join_contribution`'s "never on join
                        // spelling alone" guarantee; an empty declared
                        // `unique_key` leaves the `JoinContext` with no entry
                        // for the alias, which `fan_out` already treats as
                        // fail-closed `OneToMany` (never an optimistic
                        // empty-key-set match).
                        if let Some(alias) = join_shape::join_alias_for_source(inputs.sql, source) {
                            // join-context: builder (single-source context for this column's own fold-contribution check, not a shared route context)
                            let mut join_ctx = JoinContext::new();
                            if !facts.unique_key.is_empty() {
                                let key_cols: Vec<&str> =
                                    facts.unique_key.iter().map(String::as_str).collect();
                                join_ctx = join_ctx.with_composite_unique_key(&alias, &key_cols);
                            }
                            let mut retractable_columns = Vec::new();
                            let mut contribution_reason: Option<String> = None;
                            for (column, combiner) in &fold.add_columns {
                                let column_sensitive_to_source =
                                    inputs.column_groups.iter().any(|g| {
                                        g.mutation_sensitivity.contains(source)
                                            && g.columns.contains(column)
                                    });
                                if !column_sensitive_to_source {
                                    continue;
                                }
                                let Some(cardinality) = join_shape::source_join_cardinality(
                                    inputs.sql, source, &join_ctx,
                                ) else {
                                    continue;
                                };
                                let discriminants = combiner_discriminants(*combiner, false);
                                if let ContributionVerdict::Refused(reason) =
                                    join_contribution_monotone(cardinality, &discriminants)
                                {
                                    retractable_columns.push(column.clone());
                                    contribution_reason.get_or_insert(reason);
                                }
                            }
                            if !retractable_columns.is_empty() {
                                plan.refusals.push(Refusal::KeyedRetractableContribution {
                                    source: source.to_string(),
                                    columns: retractable_columns,
                                    why: format!(
                                        "{}; the repair family also cannot admit a per-group \
                                         recompute for the retraction: {repair_why}",
                                        contribution_reason.unwrap_or_default()
                                    ),
                                });
                            }
                        }
                    }
                }
                return;
            }

            // Condition (2): combiner algebra class, checked independently
            // of the (already-passed) source-posture condition above, per
            // column — a mixed-combiner fold refuses as a whole (fail-closed,
            // not a partial fold) the moment any one column's combiner is
            // not a monoid.
            for (column, combiner) in &fold.add_columns {
                if let FaithfulFold::Fails {
                    submultiset_fold: ConditionVerdict::Fails { reason },
                    ..
                } = faithful_fold(*combiner, false, &posture, discovery)
                {
                    // The once-write family (`COALESCE`,
                    // `incremental_shapes.md` §"The column-family
                    // catalogue") is not a commutative monoid and not
                    // order-monotone either, so this ALGEBRA leg — and only
                    // this leg — would fail-closed-refuse it. Its admission
                    // rests on an INDEPENDENT proof already verified by the
                    // SAME shared helper the runtime classifier uses
                    // (`rules::cumulative::classify_once_write`:
                    // key-derived, or a declared `key -> <source column>`
                    // functional dependency not structurally disproven —
                    // `smelt_db::queries::maintenance::derive_fold_spec`
                    // only ever puts a `Coalesce` column into a `FoldSpec`
                    // after that proof passes), so this stage does not
                    // re-derive it. The waiver is scoped to the algebra
                    // verdict: the source-posture / delta-discovery
                    // condition is combiner-independent and already binds
                    // above (the `representative` check), and the
                    // snapshot-reconcile run-shape gate below binds too.
                    if *combiner == SqlFunction::Coalesce {
                        continue;
                    }
                    plan.refusals.push(Refusal::NoAdmissibleTechnique {
                        trigger: format!("{trigger:?}"),
                        why: format!("combiner {combiner:?} for column '{column}' {reason}"),
                    });
                    return;
                }
            }
            // Run-shape gate (`docs/specs/incremental_shapes.md` §"The two
            // run shapes"; plan/classifier agreement with
            // `rules::cumulative::classify_cumulative`'s
            // `KeyedSnapshotSourceUnsupportedColumn`), consulted last —
            // AFTER both faithful-fold obligations above already passed —
            // because it is an INDEPENDENT admission axis, not a competing
            // reason for the same failure: obligations 2/3 read the
            // TRIGGERING source's declared `MutationProfile`/combiner (an
            // append-only source's posture passes obligation 2 on its own,
            // clock-independent), which cannot by itself distinguish "this
            // source's deltas are a retraction-free event feed"
            // (window-forward) from "this whole model has no clocked source
            // at all and is driven by a full-snapshot rescan every run"
            // (snapshot-reconcile, `smelt-runtime::cumulative::
            // execute_snapshot_reconcile`). The run shape is a WHOLE-MODEL
            // property — zero clocked sources anywhere in the model,
            // mirroring `CumulativeClassification::is_snapshot_reconcile`
            // — never a property of the single triggering `source`, so
            // every declared source is consulted here, not just `facts`.
            // Mirroring `classify_cumulative`'s own resolution
            // (`resolve_single_anchor`) further: snapshot-reconcile is only
            // derived when the zero-clocked sources resolve to a SINGLE
            // unambiguous candidate — two or more declared sources with
            // none clocked is the distinct `KeyedSnapshotPostureUnsupported`
            // shape (an unrelated, pre-existing refusal this gate does not
            // own), not the double-count case this gate names. A model
            // already refused above (e.g. a `MutableSnapshot` source
            // failing obligation 2's posture check) keeps ITS refusal
            // reason — this gate only fires for a fold that would
            // otherwise be admitted, where under snapshot-reconcile it
            // always double-counts (or, for an extremal/order-monotone
            // combiner, computes a history observation instead of the
            // current value) no matter how clean the triggering source's
            // own posture is. Refused fail-loud, not silently skipped like
            // the no-fold arm above (a real fold specification WAS found;
            // admitting nothing without saying why would hide the gap from
            // `smelt explain`).
            if inputs.sources.len() == 1 && inputs.sources[0].partition_col.is_none() {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!(
                        "fold over '{source}' is refused under the snapshot-reconcile run \
                         shape (no clocked source anywhere in this model) — re-folding state \
                         double-counts: a mutable snapshot is not a replayable, \
                         retraction-free event feed (or, for an extremal/order-monotone \
                         combiner, computes a history observation instead of the current \
                         value). Wrap the fold column as ANY_VALUE(...) for the \
                         plain-overwrite family instead, or declare `timeseries:` on a \
                         driving source to use the window-forward run shape."
                    ),
                });
                return;
            }

            plan.cells.push(PlanCell {
                group: format!(
                    "{{{}}}",
                    fold.add_columns
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                trigger,
                corner: Corner::FoldDelta,
                technique: Technique::KeyedFold,
                // Keyed end-state: the write is key-addressed, not
                // partition-addressed; there is no partition axis to bound.
                partition_local: PartitionLocal::Yes,
                scans: vec![],
                ledger_catch_up: false,
                row_identity: identity.clone(),
                skeleton_source_closure: None,
                fingerprint_projections: BTreeMap::new(),
                key_scope: None,
                state_downgrade: None,
            });
        }
        Grain::Succession { .. } => {
            // Unreachable: a succession-grain output is derived by
            // `maintenance::succession::derive_succession_plan`, which
            // bypasses this general-purpose deriver entirely (mirroring
            // `unsupported_grain_plan`/`locality_refused_plan`'s own
            // bypass) — there is nothing meaningful to derive here.
            unreachable!(
                "Grain::Succession is derived by maintenance::succession::derive_succession_plan, \
                 never by derive_maintenance_plan"
            );
        }
    }
}
