use super::*;

/// Assemble inputs (resolved source facts, declared output shape,
/// `maintenance.cells[]`) and derive the plan, mapping its refusals into
/// [`MaintenancePlanDiagnostics`]. `source_refs` is every `smelt.<path>`
/// this model's SQL references that resolves to a source declaration
/// (already resolved by the caller — mirrors
/// `smelt-db::lib::ref_timeseries_config`'s resolution seam).
/// `extra_model_sources` is every referenced upstream model that is itself
/// a locality-admitted composed output (`grain: key` + `timeseries:`),
/// already resolved by the caller (`smelt-db::lib::ref_model_source_facts`)
/// — appended to the declared-source candidate pool `resolve_driving_source`
/// consults, paired with its own granularity folded into
/// `driving_source_granularity`'s "exactly one clocked candidate" rule, so
/// a `grain: key` model may take a composed upstream model's own output as
/// its driving source exactly as it would a declared source
/// (`incremental_shapes.md` §"Key temporal locality (the time-partitioned
/// output)" — "The output as a clocked source").
///
/// Pure function — the `#[salsa::tracked]` wrapper in `smelt-db/src/lib.rs`
/// only gathers `source_refs`/`metadata`/`sql` and calls this.
#[allow(clippy::too_many_arguments)]
pub fn maintenance_plan_diagnostics(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    source_refs: &[(String, Option<SourceInfo>)],
    project_scan_bounds: Option<&ScanBoundsConfig>,
    extra_model_sources: &[(SourceFacts, Granularity)],
    active_backends: &[String],
    warehouse_tables: smelt_core::config::WarehouseTables,
    deployed_column_names: &[String],
    deployed_model_sql: Option<&str>,
    deployed_partition_column: Option<&str>,
) -> MaintenancePlanDiagnostics {
    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let (mut sources, scan_bounds_warn_candidates) =
        build_source_facts(source_refs, model_scan_bounds, project_scan_bounds);
    for (facts, _) in extra_model_sources {
        if !sources.iter().any(|s| s.name == facts.name) {
            sources.push(facts.clone());
        }
    }
    let explicitly_mutable: std::collections::HashSet<String> = source_refs
        .iter()
        .filter(|(_, info)| {
            info.as_ref().is_some_and(|i| {
                i.mutation_profile
                    .as_ref()
                    .is_some_and(|m| m.kind == SourceMutationKind::Mutable)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();
    let granularity_mismatch = metadata
        .timeseries
        .as_ref()
        .and_then(|ts| check_declared_granularity(sql, &ts.partition_column, ts.granularity));
    let mut clocked_granularities: Vec<Granularity> = source_refs
        .iter()
        .filter_map(|(_, info)| info.as_ref().and_then(|i| i.timeseries.as_ref()))
        .map(|t| t.granularity)
        .collect();
    clocked_granularities.extend(extra_model_sources.iter().map(|(_, g)| *g));
    let driving_source_granularity = single_clocked_granularity(clocked_granularities);
    let key_recurrences = build_key_recurrences(source_refs);
    let source_referential_integrity = build_source_referential_integrity(source_refs);
    let Some(mut result) = derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        &sources,
        &explicitly_mutable,
        driving_source_granularity,
        &key_recurrences,
        // The deployed-schema snapshot is now a Salsa world-fact input
        // (`workspace_ingest::register_deployed_schemas_from_disk`) the
        // `#[salsa::tracked]` wrapper in `smelt-db/src/lib.rs` resolves and
        // passes down here — `smelt-db` itself still does no I/O, per the
        // Salsa-purity rule; it only forwards what the caller resolved.
        deployed_column_names,
        &source_referential_integrity,
        deployed_model_sql,
        deployed_partition_column,
        source_refs,
    ) else {
        return MaintenancePlanDiagnostics {
            granularity_mismatch,
            ..Default::default()
        };
    };
    // `on_violation: warn` (`incremental_models.md` §"Partition-local
    // maintenance (the K8 guardrail)"): a source in `scan_bounds_warn_
    // candidates` is only a REAL violation when the first pass actually
    // refused it with `ScanUnbounded` — a candidate whose scan turned out
    // to be bounded anyway (e.g. the driving, already-clocked source) must
    // not be reported. Only for the sources that genuinely refused, re-
    // derive once more with `allow_full_scan` forced on for exactly those
    // sources, admitting the plan and surfacing each as a Warning instead
    // of a refusal.
    let scan_bounds_warnings: Vec<String> = result
        .plan
        .refusals
        .iter()
        .filter_map(|r| match r {
            smelt_logical::maintenance::Refusal::ScanUnbounded { source, .. }
                if scan_bounds_warn_candidates.contains(source) =>
            {
                Some(source.clone())
            }
            _ => None,
        })
        .collect();
    if !scan_bounds_warnings.is_empty() {
        for facts in sources.iter_mut() {
            if scan_bounds_warnings.contains(&facts.name) {
                facts.allow_full_scan = true;
            }
        }
        if let Some(admitted) = derive_model_maintenance_plan(
            sql,
            table,
            metadata,
            &sources,
            &explicitly_mutable,
            driving_source_granularity,
            &key_recurrences,
            deployed_column_names,
            &source_referential_integrity,
            deployed_model_sql,
            deployed_partition_column,
            source_refs,
        ) {
            result = admitted;
        }
    }
    let refusals = result
        .plan
        .refusals
        .iter()
        .filter_map(|r| match r {
            smelt_logical::maintenance::Refusal::ScanUnbounded { source, why } => {
                Some(MaintenanceRefusal::ScanUnbounded {
                    source: source.clone(),
                    why: why.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::NoAdmissibleTechnique { trigger, why } => {
                Some(MaintenanceRefusal::NoAdmissibleTechnique {
                    trigger: trigger.clone(),
                    why: why.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::SkeletonChanged { column } => {
                Some(MaintenanceRefusal::SkeletonChanged {
                    column: column.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::SkeletonClauseChanged { reason } => {
                Some(MaintenanceRefusal::SkeletonClauseChanged {
                    reason: reason.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::PartitionColumnChanged { from, to } => {
                Some(MaintenanceRefusal::PartitionColumnChanged {
                    from: from.clone(),
                    to: to.clone(),
                })
            }
            // An underivable upstream-model clock. Recorded in the plan (and
            // surfaced by `smelt explain`'s Refusals section), but not yet
            // folded into `file_diagnostics()` — `MaintenanceReachNotDerivable`
            // has no `DiagnosticCode` variant yet (`diagnostics.md` §Known
            // divergences). Leave unmapped so a future phase's own diagnostic
            // lands it, exactly as `SkeletonChanged` above.
            smelt_logical::maintenance::Refusal::ReachNotDerivable { .. } => None,
            smelt_logical::maintenance::Refusal::UnsupportedGrain {
                grain,
                tracking_plan,
            } => Some(MaintenanceRefusal::UnsupportedGrain {
                grain: grain.clone(),
                tracking_plan: tracking_plan.clone(),
            }),
            smelt_logical::maintenance::Refusal::LocalityNotEstablished { message } => {
                Some(MaintenanceRefusal::LocalityNotEstablished {
                    message: message.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::KeyedRecurrenceDeclarationMismatch { message } => {
                Some(MaintenanceRefusal::KeyedRecurrenceDeclarationMismatch {
                    message: message.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::IdentityNotDerivable { message } => {
                Some(MaintenanceRefusal::IdentityNotDerivable {
                    message: message.clone(),
                })
            }
            // The repair family's two obligation refusals
            // (`MaintenanceRepairKeysNotDiscoverable`/
            // `MaintenanceRepairSliceUnbounded`) — `derive_new_data`
            // (`smelt-logical/src/maintenance/derive.rs`) already pushes
            // both when `repair::admit_per_group_recompute` refuses, but
            // neither has a `DiagnosticCode` variant yet. Left unmapped
            // exactly as `ReachNotDerivable` above, for the same reason: a
            // future phase's own diagnostic lands it.
            smelt_logical::maintenance::Refusal::RepairKeysNotDiscoverable { .. } => None,
            smelt_logical::maintenance::Refusal::RepairSliceUnbounded { .. } => None,
            smelt_logical::maintenance::Refusal::DefinitionChangeNotBackfillable {
                columns,
                why,
            } => Some(MaintenanceRefusal::DefinitionChangeNotBackfillable {
                columns: columns.clone(),
                why: why.clone(),
            }),
            smelt_logical::maintenance::Refusal::KeyedRetractableContribution {
                source,
                columns,
                why,
            } => Some(MaintenanceRefusal::KeyedRetractableContribution {
                source: source.clone(),
                columns: columns.clone(),
                why: why.clone(),
            }),
            smelt_logical::maintenance::Refusal::SuccessionNotRecognized { reason } => {
                Some(MaintenanceRefusal::SuccessionNotRecognized {
                    reason: reason.clone(),
                })
            }
        })
        .collect();
    let cell_column_group_violations = metadata
        .maintenance
        .as_ref()
        .map(|m| cell_column_group_violations(m, &result.column_groups))
        .unwrap_or_default();
    let write_pin_refusals = write_pin_diagnostics(
        metadata,
        &result.plan,
        &result.column_groups,
        active_backends,
        &result.comparability,
    );
    // Availability resolution for the two state-residency diagnostics
    // (`state.md` §Diagnostics `MaintenanceStateDowngraded` /
    // `DeclaredContractRequiresState`). Runs over a CLONE of the derived
    // cells — `result.plan` itself must stay ideal-derivation output, since
    // `smelt-runtime` and `smelt explain` resolve availability themselves
    // against the actual target dialect (plan 05/06's own posture: analysis
    // time has no single declared target). Checked against every declared
    // backend, the same all-declared-backends posture `write_pin_diagnostics`
    // uses; an empty `active_backends` (config unparseable) falls back to
    // `duckdb`, mirroring that function's own fallback.
    let availability_backends: Vec<String> = if active_backends.is_empty() {
        vec!["duckdb".to_string()]
    } else {
        active_backends.to_vec()
    };
    let realisable_for =
        |backend_name: &str| -> Vec<smelt_logical::maintenance::availability::StateStructure> {
            backend_dialect_for(backend_name)
                .map(smelt_logical::maintenance::availability::realisable_state_structures)
                .unwrap_or_default()
        };
    let mut state_downgrades: Vec<StateDowngradeDiagnostic> = Vec::new();
    for backend_name in &availability_backends {
        let realisable = realisable_for(backend_name);
        let availability = smelt_logical::maintenance::availability::StateAvailability::resolve(
            warehouse_tables,
            &realisable,
        );
        let mut cells = result.plan.cells.clone();
        smelt_logical::maintenance::availability::resolve_availability(&mut cells, &availability);
        for cell in &cells {
            let Some(downgrade) = &cell.state_downgrade else {
                continue;
            };
            let cell_label = format!("{:?}", cell.trigger);
            if state_downgrades.iter().any(|d| d.cell == cell_label) {
                continue;
            }
            state_downgrades.push(StateDowngradeDiagnostic {
                cell: cell_label,
                original_technique: format!("{:?}", downgrade.original),
                missing_structure: downgrade.missing.as_str().to_string(),
                backend: backend_name.clone(),
                reason: downgrade.reason.clone(),
            });
        }
    }
    let mut contract_state_refusals: Vec<ContractStateRefusalDiagnostic> = Vec::new();
    if let Some(contract) = metadata.contract.as_ref() {
        let mut declarations: Vec<String> = Vec::new();
        if contract.deferral.is_some() {
            declarations.push("contract.deferral".to_string());
        }
        for cell_cfg in &contract.cells {
            if cell_cfg.deferral.is_some() {
                declarations.push(format!("contract.cells[].deferral (on: {})", cell_cfg.on));
            }
        }
        if !declarations.is_empty() {
            // The concrete `d` value never changes which structure a
            // `Deferral` point requires (`required_state_structure` dispatches
            // on the variant, not its payload) — `0` is a placeholder.
            let point = smelt_logical::contract::ContractPoint::Deferral { d: 0 };
            if let Some(required) = smelt_logical::contract::required_state_structure(&point) {
                for declaration in declarations {
                    for backend_name in &availability_backends {
                        let realisable = realisable_for(backend_name);
                        let availability =
                            smelt_logical::maintenance::availability::StateAvailability::resolve(
                                warehouse_tables,
                                &realisable,
                            );
                        if !availability.contains(required) {
                            contract_state_refusals.push(ContractStateRefusalDiagnostic {
                                declaration: declaration.clone(),
                                missing_structure: required.as_str().to_string(),
                                backend: backend_name.clone(),
                            });
                            break;
                        }
                    }
                }
            }
        }
    }
    MaintenancePlanDiagnostics {
        refusals,
        cell_column_group_violations,
        granularity_mismatch,
        write_pin_refusals,
        scan_bounds_warnings,
        state_downgrades,
        contract_state_refusals,
        succession_advisories: result.succession_advisories,
    }
}
