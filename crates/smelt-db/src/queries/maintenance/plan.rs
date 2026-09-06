use super::*;

/// Assemble [`ModelInputs`] from already-resolved facts and derive the
/// plan. `sql` is the model body with frontmatter stripped. `sources` is
/// every source the model's `FROM`/`JOIN` clauses reference, already
/// resolved by the caller (mirrors `smelt-db::lib::ref_timeseries_config`'s
/// resolution, reused here for `mutation_profile` /
/// `allow_full_scan` instead of just `timeseries`).
///
/// Returns `None` when the model has no maintenance plan to derive: only
/// `refresh: incremental` models carry one (`incremental_models.md` §Surface
/// "The plan (derived, reported)": "Every non-`full` model has a
/// maintenance plan").
/// `driving_source_granularity` is the model's driving source's own declared
/// granularity, when the caller can determine it (used only by a `grain:
/// key` model that also declares its own `timeseries:` block, to check the
/// key-temporal-locality gate's granularity-equality structural
/// precondition — `incremental_shapes.md` §"Key temporal locality").
/// `None` fails that precondition closed (an unproven match is never
/// admitted); the runtime execution path (`smelt-runtime::cumulative`,
/// which has the driving source's `TimeseriesConfig` directly from the
/// classifier) is today's actual consumer of an admitted route.
/// `key_recurrences` is every referenced source's declared `key_recurrence`
/// bound (`sources.md` §"`mutation_profile` — the structured block"), keyed
/// by bare source name (the same convention `SourceFacts::name` and
/// `resolve_driving_source`'s resolved `driving.name` use) — consulted only
/// by key temporal locality's route 3 (recurrence-bounded) as the declared
/// fallback when no bound is statically derivable from the model's own SQL
/// (`docs/specs/incremental_shapes.md` §"Key temporal locality"). Build via
/// [`build_key_recurrences`], the sibling of [`build_source_facts`] over the
/// same `(ref_string, source_info)` pairs.
/// `deployed_column_names` is the model's previously-deployed output
/// column names (world-fact, read by the caller from the deployed-schema
/// snapshot the runtime's `schema_evolution` module already consults —
/// `smelt-db` itself does no I/O, per the Salsa-purity rule). An empty
/// slice means "no known deployed schema" and derives no `Trigger::
/// ColumnAdded` at all — the same fail-closed posture as before this
/// parameter existed (`docs/specs/definition_deltas.md` §"The verdict per column group"); every existing `smelt-db`-internal caller
/// (diagnostics, `smelt explain`) has no such snapshot to hand and passes
/// `&[]` unchanged. `smelt-runtime`'s maintenance driver is the one caller
/// with real I/O access to the deployed-schema store, and is the only one
/// that ever supplies a non-empty slice.
/// `source_referential_integrity` is every referenced source's declared
/// `referential_integrity` world-fact (`sources.md` §"Referential
/// integrity"), keyed by bare source name — threaded into every
/// `UpstreamMutation` cell's P1 skeleton-source-closure proof exactly like
/// [`derive_maintenance_plan_with_referential_integrity`] does. An empty map
/// (the caller's own default when it has not resolved the declaration)
/// behaves byte-identically to this function's behaviour before this
/// parameter existed — this only *adds* closure attempts for the sources
/// the caller names.
#[allow(clippy::too_many_arguments)]
pub fn derive_model_maintenance_plan(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &std::collections::HashSet<String>,
    driving_source_granularity: Option<Granularity>,
    key_recurrences: &[(String, smelt_core::sources::KeyRecurrence)],
    deployed_column_names: &[String],
    source_referential_integrity: &SourceReferentialIntegrity,
    deployed_model_sql: Option<&str>,
    deployed_partition_column: Option<&str>,
    // The `(ref, SourceInfo)` pairs the keyed-succession classifier's
    // `SuccessionContext` is built from (`build_succession_context`) — a
    // side channel alongside `sources: &[SourceFacts]`, consulted only when
    // `metadata.resolved_grain()` is `None`. An empty slice degrades to the
    // classifier's own fail-closed refusal (`SingleSourceOnly`/
    // `DrivingSourceNotAppendOnly`), never a panic — callers with no source
    // declarations in scope (most `smelt-runtime` execution-path callers,
    // pre-succession) pass `&[]`.
    source_refs: &[(String, Option<SourceInfo>)],
) -> Option<MaintenancePlanResult> {
    if metadata.refresh != Some(RefreshStrategy::Incremental) {
        return None;
    }
    // The declared `grain:` check-only assertion when written (already
    // validated against the declared facts by
    // `smelt_core::metadata::validate_timeseries`), otherwise the label
    // derived from the two shape-defining facts (`timeseries:` /
    // `unique_key:`) — `docs/specs/models.md` §"Refresh axis". Reading the
    // resolved label here (rather than the raw `grain` field) is what admits
    // `refresh: incremental` on the facts alone, with no `grain:` written.
    //
    // `None` here (no declared/derivable `timeseries:`/`unique_key:`) is no
    // longer "not incremental" — it's the keyed-succession grain's own
    // undeclared-admission shape (`docs/specs/incremental_shapes.md`
    // §"Succession-grain admission (no declaration)"): the leaf classifier
    // decides admission on the model's own SQL, never a declared grain.
    let Some(grain) = metadata.resolved_grain() else {
        let ctx = build_succession_context(sql, source_refs);
        let verdict = match smelt_logical::analysis::walk::QueryTree::from_sql(sql) {
            Some(tree) => smelt_logical::analysis::walk::model_keyed_succession(&tree, &ctx),
            None => smelt_logical::analysis::succession::SuccessionVerdict::NotSuccession {
                reason:
                    smelt_logical::analysis::succession::NotSuccessionReason::PatternUnrecognized(
                        "SQL has no SELECT statement".to_string(),
                    ),
            },
        };
        let derivation =
            smelt_logical::maintenance::succession::derive_succession_plan(&verdict, table);
        return Some(MaintenancePlanResult {
            plan: derivation.plan,
            column_groups: Vec::new(),
            degenerate: Vec::new(),
            state_columns: Vec::new(),
            execution_postures: None,
            is_snapshot_reconcile: None,
            comparability: Vec::new(),
        });
    };
    if grain == ConfigGrain::KeyPerPartition {
        // Not yet supported: deriving a real plan for `key_per_partition`
        // needs trajectory/backfill machinery that doesn't exist yet
        // (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
        // Phase A0). Refuse fail-loud instead of silently collapsing into a
        // keyed plan with an empty `unique_key` — there is nothing
        // meaningful to derive here, so this bypasses
        // `derive_maintenance_plan` entirely rather than feeding it inputs
        // built from a grain it was never taught to admit.
        return Some(MaintenancePlanResult {
            plan: smelt_logical::maintenance::unsupported_grain_plan("key_per_partition"),
            column_groups: Vec::new(),
            degenerate: Vec::new(),
            state_columns: Vec::new(),
            execution_postures: None,
            is_snapshot_reconcile: None,
            comparability: Vec::new(),
        });
    }
    let partition_col = metadata
        .timeseries
        .as_ref()
        .map(|t| t.partition_column.clone());
    // The admitted key-temporal-locality verdict for a `grain: key` model
    // that also declares a `timeseries:` block — captured here (the `Ok`
    // branch of `establish_locality`, below) and folded onto the derived
    // plan's `key_locality` after `derive_maintenance_plan` runs, so
    // `smelt-db`'s diagnostics and `smelt explain` can read the
    // already-admitted verdict instead of re-deriving it
    // (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    // Phase A5).
    let mut established_key_locality: Option<smelt_logical::maintenance::locality::LocalitySlice> =
        None;
    let plan_grain = match grain {
        ConfigGrain::Partition => PlanGrain::Partition {
            partition_col: partition_col.clone().unwrap_or_default(),
        },
        ConfigGrain::Key => {
            // The model's real derived `unique_key` — the GROUP BY columns
            // of its own outermost SELECT (the same derivation the keyed
            // classifier, `rules::cumulative::classify_cumulative`,
            // performs) — rather than a hardcoded empty vec. Threading it
            // here does not change which techniques any existing plan
            // admits: `derive_maintenance_plan`'s admission logic does not
            // yet branch on `Grain::Key`'s `unique_key` contents.
            let unique_key = derive_group_by_unique_key(sql);
            // A declared top-level `unique_key:` (`docs/specs/models.md`
            // §"Refresh axis") must agree with the GROUP-BY-derived key —
            // never a silent preference for either list
            // (`models.md` §"Constraint violations": "For aggregated key
            // bodies: `unique_key` ≠ the `GROUP BY` column set → hard error
            // (checked restatement)"). A model with no declared top-level
            // `unique_key:` (the pre-existing surface, relying on the
            // GROUP-BY derivation alone) has nothing to check against.
            if let Some(declared) = metadata.unique_key.as_deref() {
                if let Err((declared, derived)) = declared_unique_key_matches(declared, sql) {
                    return Some(MaintenancePlanResult {
                        plan: locality_refused_plan(format!(
                            "model '{table}' declares unique_key: {declared:?} but its \
                             outermost SELECT's GROUP BY derives {derived:?} — the declared \
                             identity must restate the GROUP BY column set exactly \
                             (docs/specs/models.md §\"Constraint violations\")"
                        )),
                        column_groups: Vec::new(),
                        degenerate: Vec::new(),
                        state_columns: Vec::new(),
                        execution_postures: None,
                        is_snapshot_reconcile: None,
                        comparability: Vec::new(),
                    });
                }
            } else if unique_key.is_empty() {
                // No declared top-level `unique_key:` and the model's own
                // GROUP BY derives no key either — there is no identity to
                // check anything against. Checked here (frontmatter-time,
                // reached by `file_diagnostics()` and `smelt explain`
                // without a run) rather than left to fail later, opaquely,
                // wherever a plan first consults `unique_key`
                // (`docs/specs/models.md` §"Constraint violations").
                return Some(MaintenancePlanResult {
                    plan: identity_not_derivable_plan(format!(
                        "model '{table}' asserts grain: key but declares no top-level \
                         unique_key: and its outermost SELECT's GROUP BY derives no key \
                         (empty) — a keyed model must have a derivable identity, either a \
                         declared unique_key: or a non-empty GROUP BY \
                         (docs/specs/models.md §\"Constraint violations\")"
                    )),
                    column_groups: Vec::new(),
                    degenerate: Vec::new(),
                    state_columns: Vec::new(),
                    execution_postures: None,
                    is_snapshot_reconcile: None,
                    comparability: Vec::new(),
                });
            }
            // A `grain: key` model that also declares a `timeseries:`
            // block must clear the key-temporal-locality gate before a
            // plan is derived at all — the single entry point deciding
            // keyed+timeseries admissibility
            // (`smelt_logical::maintenance::locality::establish_locality`,
            // `docs/specs/incremental_shapes.md` §"Key temporal locality").
            if let Some(own_ts) = metadata.timeseries.as_ref() {
                // The driving source is the single alias-scoped FROM/JOIN
                // input that both is a referenced source and declares its
                // own `timeseries:` clock — resolved by the shared
                // `locality::resolve_driving_source` helper, the same
                // anchor resolution `classify_cumulative` uses at runtime
                // (`smelt_logical::maintenance::locality::
                // resolve_driving_source`'s doc comment), so this static
                // plan-derivation call site and the runtime execution path
                // (`smelt-runtime::cumulative`) agree on which source drives
                // the model rather than each resolving it independently.
                // Neither "no clocked candidate" nor "ambiguous" (more than
                // one alias-scoped candidate) resolve a driving source here;
                // both fail the gate's structural preconditions closed.
                let (
                    driving_source_name,
                    driving_source_has_clock,
                    driving_source_partition_column,
                ) = match smelt_logical::maintenance::locality::resolve_driving_source(sql, sources)
                {
                    Ok(Some(driving)) => {
                        (driving.name.clone(), true, driving.partition_col.clone())
                    }
                    Ok(None) | Err(_) => (String::new(), false, None),
                };
                let partition_column_not_null = partition_column_provably_not_null(
                    sql,
                    &unique_key,
                    &own_ts.partition_column,
                    driving_source_partition_column.as_deref(),
                );
                let driving_source_key_recurrence = key_recurrences
                    .iter()
                    .find(|(name, _)| name == &driving_source_name)
                    .map(|(_, kr)| kr);
                let inputs = LocalityInputs {
                    model_name: table.to_string(),
                    unique_key: unique_key.clone(),
                    partition_column: own_ts.partition_column.clone(),
                    granularity: own_ts.granularity,
                    partition_column_not_null,
                    driving_source_name,
                    driving_source_has_clock,
                    driving_source_granularity,
                    driving_source_partition_column,
                    declared_functional_dependencies: &metadata.functional_dependencies,
                    driving_source_key_recurrence,
                    sql,
                };
                match establish_locality(&inputs) {
                    Err(
                        refusal @ smelt_logical::maintenance::locality::LocalityRefusal::RecurrenceDeclarationMismatch {
                            ..
                        },
                    ) => {
                        return Some(MaintenancePlanResult {
                            plan: recurrence_mismatch_plan(refusal.message(table)),
                            column_groups: Vec::new(),
                            degenerate: Vec::new(),
                            state_columns: Vec::new(),
                            execution_postures: None,
                            is_snapshot_reconcile: None,
                            comparability: Vec::new(),
                        });
                    }
                    Err(refusal) => {
                        return Some(MaintenancePlanResult {
                            plan: locality_refused_plan(refusal.message(table)),
                            column_groups: Vec::new(),
                            degenerate: Vec::new(),
                            state_columns: Vec::new(),
                            execution_postures: None,
                            is_snapshot_reconcile: None,
                            comparability: Vec::new(),
                        });
                    }
                    // Admitted: the derived `LocalitySlice` is folded onto
                    // the plan's `key_locality` below (after
                    // `derive_maintenance_plan` runs) rather than
                    // discarded — `smelt-db`'s diagnostics and `smelt
                    // explain` are consumers of the same admitted verdict
                    // the runtime execution path (`smelt-runtime::
                    // cumulative`) already slice-prunes with, not a second
                    // re-derivation of it.
                    Ok(slice) => established_key_locality = Some(slice),
                }
            }
            PlanGrain::Key { unique_key }
        }
        ConfigGrain::KeyPerPartition => unreachable!("handled above"),
    };
    let skeleton = skeleton_columns(sql, &[], partition_col.as_deref());
    let grouping = derive_column_groups(sql, sources, &skeleton);
    let fold = match grain {
        ConfigGrain::Key => derive_fold_spec(sql, &metadata.functional_dependencies),
        _ => None,
    };
    let output = OutputSpec {
        table: table.to_string(),
        grain: plan_grain,
        skeleton_columns: skeleton,
    };
    // The definition-change trigger's inputs: `None`/unclassifiable and
    // "no deployed snapshot supplied" both fall back to "no old columns, no
    // added columns" — fail-closed, never a guessed `ColumnAdded` trigger
    // (`definition_deltas.md` §"The verdict per column group").
    let (old_columns, added_columns) = if deployed_column_names.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        smelt_logical::maintenance::derive::diff_deployed_columns(sql, deployed_column_names)
            .unwrap_or_default()
    };
    // A keyed-grain output's declared `timeseries.partition_column` — the
    // axis the footprint question is posed against (`model_properties.md`
    // §"Footprint reflection / bounded write footprint"). `None` for a
    // `Grain::Partition` output (posed against its own partition axis
    // instead) or a keyed output with no declared `timeseries:` block.
    let keyed_time_axis = match &output.grain {
        PlanGrain::Key { .. } => metadata
            .timeseries
            .as_ref()
            .map(|t| t.partition_column.as_str()),
        PlanGrain::Partition { .. } => None,
        // Unreachable: this branch of `derive_model_maintenance_plan` only
        // ever builds a `PlanGrain::Partition`/`PlanGrain::Key` output — a
        // succession-grain output is derived by the separate
        // `maintenance::succession::derive_succession_plan` path, which
        // bypasses this code entirely (see the `resolved_grain()`-is-`None`
        // branch above).
        PlanGrain::Succession { .. } => unreachable!(
            "PlanGrain::Succession is derived by maintenance::succession::derive_succession_plan, \
             never by this branch of derive_model_maintenance_plan"
        ),
    };
    let inputs = ModelInputs {
        sql,
        output,
        sources: sources.to_vec(),
        column_groups: grouping.groups.clone(),
        fold,
        old_columns,
        old_sql: deployed_model_sql,
        keyed_time_axis,
        old_partition_col: deployed_partition_column,
    };

    // Trigger derivation itself is a pure `smelt-logical` function
    // (`derive::derive_triggers`, `incremental_models.md` §"Per-cell
    // admission" → "Which changed inputs get a mutation cell") — this
    // wrapper only assembles the facts (Salsa purity rule).
    let triggers = smelt_logical::maintenance::derive::derive_triggers(
        sources,
        &grouping.groups,
        explicitly_mutable,
        &added_columns,
    );

    let mut plan = derive_maintenance_plan_with_referential_integrity(
        &inputs,
        &triggers,
        source_referential_integrity,
    );
    plan.key_locality = established_key_locality.map(|slice| {
        let bound = smelt_logical::maintenance::locality::settle_bound(&slice);
        smelt_logical::maintenance::KeyLocality {
            slice,
            settle_bound: bound,
        }
    });
    // The single `model_property_vector` call this derivation surfaces to
    // callers (`MaintenancePlanResult::comparability`'s own doc comment) —
    // `derive_fold_spec` above already re-derives the same vector for a
    // `grain: key` model's fold-spec walk, so this call is only load-bearing
    // for a `grain: partition` model (no fold spec) or when the fold-spec
    // walk itself failed to parse; either way, consumers read this field,
    // never re-walk.
    let comparability = smelt_logical::analysis::walk::model_property_vector(
        sql,
        &smelt_logical::analysis::join_shape::JoinContext::new(),
    )
    .map(|v| v.comparability)
    .unwrap_or_default();
    Some(MaintenancePlanResult {
        plan,
        column_groups: grouping.groups,
        degenerate: grouping.degenerate,
        comparability,
        state_columns: Vec::new(),
        execution_postures: None,
        is_snapshot_reconcile: None,
    })
}

/// Like [`derive_model_maintenance_plan`], but additionally folds the
/// creation-trigger cells (and `MaintenanceReachNotDerivable` refusals) for
/// the model's **upstream maintained-model edges** into the plan
/// (`incremental_models.md` §"Upstream model edges").
///
/// `model_edges` is assembled by the caller from each upstream model's own
/// already-validated metadata (the leading `smelt.` stripped from the ref
/// name; `clock_col` from the upstream's `timeseries.partition_column`, or
/// `None` when it declares none). View/`full` upstreams deliver no
/// incremental delta and must not appear here — the caller excludes them, so
/// they contribute neither a creation cell nor a refusal.
///
/// Kept as a wrapper over [`derive_model_maintenance_plan`] so the many
/// source-only callers (`smelt-runtime`'s maintenance driver and propagation
/// walk) are unchanged; both entry points still call one pure derivation.
#[allow(clippy::too_many_arguments)]
pub fn derive_model_maintenance_plan_with_edges(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &std::collections::HashSet<String>,
    model_edges: &[smelt_logical::maintenance::derive::ModelEdge],
    driving_source_granularity: Option<Granularity>,
    key_recurrences: &[(String, smelt_core::sources::KeyRecurrence)],
    deployed_column_names: &[String],
    source_referential_integrity: &SourceReferentialIntegrity,
    deployed_model_sql: Option<&str>,
    deployed_partition_column: Option<&str>,
    source_refs: &[(String, Option<SourceInfo>)],
) -> Option<MaintenancePlanResult> {
    let mut result = derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        driving_source_granularity,
        key_recurrences,
        deployed_column_names,
        source_referential_integrity,
        deployed_model_sql,
        deployed_partition_column,
        source_refs,
    )?;
    // Model edges only clamp against a partition-addressed output axis; a
    // key-addressed downstream contributes none (deferred). Reads the
    // resolved (declared-or-derived) grain, matching `derive_model_maintenance_plan`
    // above, so a facts-alone partition-grain model (no `grain:` written)
    // clamps the same way as one that writes `grain: partition` explicitly.
    let output_partition_col = match metadata.resolved_grain() {
        Some(ConfigGrain::Partition) => metadata
            .timeseries
            .as_ref()
            .map(|t| t.partition_column.as_str()),
        _ => None,
    };
    smelt_logical::maintenance::derive::append_model_edge_cells(
        &mut result.plan,
        sql,
        output_partition_col,
        model_edges,
        metadata.unique_key.as_deref().unwrap_or(&[]),
        sources,
        source_referential_integrity,
    );
    Some(result)
}
