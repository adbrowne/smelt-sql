//! `--dry-run` outcome construction — split out of `project.rs` so the
//! per-model execute loop's file stays under the large-file ratchet.
//!
//! When `request.dry_run` is set, `execute_project` resolves the execution
//! strategy per model from graph config alone, compiles each model's SQL
//! (full-refresh form only), emits `reporter.model_compiled` for each, and
//! returns the plan without invoking `BackendFactory::create` or executing
//! any SQL.
//!
//! Parity rule: the reporter callback is the only place compiled SQL is
//! surfaced to the consumer. CLI/UI must not re-implement compilation after
//! this returns. See `docs/specs/architecture.md` §"Run pipeline parity rule".

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn build_dry_run_outcome(
    run_id: &str,
    run_start: chrono::DateTime<Utc>,
    request: &ExecuteRequest,
    config: &Config,
    graph_lock: &DependencyGraph,
    db: &Arc<tokio::sync::Mutex<smelt_db::Database>>,
    project_dir: &Path,
    reporter: &dyn RunReporter,
    fn_bodies: crate::fn_bodies::FnBodyMap,
    selected: &[String],
    source_timeseries: &smelt_planner::SourceTimeseriesMap,
    source_infos: &[smelt_core::sources::SourceInfo],
    state_availability: &HashMap<
        String,
        smelt_logical::maintenance::availability::StateAvailability,
    >,
    model_plans: &[ModelPlan],
) -> Result<RunOutcome> {
    let summary_models: Vec<ModelPlanRecord> = selected
        .iter()
        .map(|model_name| {
            let strategy = if let Ok(model) = graph_lock.get_model(model_name) {
                let metadata = model.metadata.as_deref();
                let frontmatter = Frontmatter::parse(&model.content);
                let materialization =
                    config.get_materialization_with_metadata(model_name, metadata);
                let inc_config = config
                    .get_incremental_with_metadata(model_name, metadata)
                    .or_else(|| frontmatter.as_ref().and_then(|f| f.batched_config()));
                let ts_config = config
                    .get_timeseries_with_metadata(model_name, metadata)
                    .cloned()
                    .or_else(|| metadata.and_then(|m| m.timeseries.clone()));

                // Route keyed detection through is_keyed().
                if metadata.is_some_and(|m| m.is_keyed()) {
                    ModelStrategy::Keyed
                } else {
                    match materialization {
                        smelt_core::config::Materialization::Ephemeral => ModelStrategy::Ephemeral,
                        _ => match (
                            inc_config,
                            ts_config,
                            request.start.as_deref(),
                            request.end.as_deref(),
                        ) {
                            (Some(_inc), Some(ts), Some(_), Some(_)) => {
                                ModelStrategy::Incremental {
                                    partition_column: ts.partition_column.clone(),
                                    granularity: format!("{:?}", ts.granularity).to_lowercase(),
                                }
                            }
                            _ => ModelStrategy::FullRefresh,
                        },
                    }
                }
            } else {
                ModelStrategy::FullRefresh
            };

            let materialization = graph_lock
                .get_model(model_name)
                .ok()
                .map(|m| {
                    config.get_materialization_with_metadata(model_name, m.metadata.as_deref())
                })
                .unwrap_or(smelt_core::config::Materialization::View);

            let dependencies = graph_lock.get_upstream(model_name);

            ModelPlanRecord {
                name: model_name.clone(),
                strategy,
                materialization,
                dependencies,
            }
        })
        .collect();

    let plan_summary = PlanSummary {
        models: summary_models,
    };

    // ── Dry-run compile: build minimal CompilerRegistry + EphemeralResolver
    // so we can emit model_compiled callbacks without requiring a backend.
    // We compile the full-refresh form (no time-filter injection) because
    // dry-run is about showing what SQL *would* run, not a specific batch.
    {
        let needed_targets_dry: HashSet<String> = selected
            .iter()
            .filter_map(|name| {
                graph_lock
                    .get_model(name)
                    .ok()
                    .map(|m| config.get_target(name, m.metadata.as_deref(), &request.target))
            })
            .collect();

        let needed_target_configs_dry: HashMap<String, smelt_core::config::Target> =
            needed_targets_dry
                .iter()
                .filter_map(|t| config.targets.get(t).map(|c| (t.clone(), c.clone())))
                .collect();

        let mut compilers_dry = CompilerRegistry::new(config, &needed_target_configs_dry);

        let all_models_dry: Vec<smelt_core::ModelFile> =
            graph_lock.iter_models().map(|(_, m)| m.clone()).collect();

        // Build UpstreamSchemas from the Salsa DB.
        if let Ok(upstream) = {
            let db_guard = db.lock().await;
            let db_ref: &smelt_db::Database = &db_guard;
            UpstreamSchemas::from_database(db_ref, project_dir, &all_models_dry)
        } {
            compilers_dry.set_upstream_schemas_all(Arc::new(upstream));
        }
        compilers_dry.set_state_bearing_models_all(build_state_bearing_models(
            &all_models_dry,
            source_timeseries,
        ));
        if !fn_bodies.is_empty() {
            compilers_dry.set_function_bodies_all(fn_bodies);
        }

        // Build ephemeral resolvers.
        let mut ephemerals_by_target_dry: HashMap<String, Vec<(String, String)>> = HashMap::new();
        {
            let exec_order = graph_lock.execution_order().unwrap_or_default();
            for mn in &exec_order {
                let Ok(mf) = graph_lock.get_model(mn) else {
                    continue;
                };
                let md = mf.metadata.as_deref();
                let mat = config.get_materialization_with_metadata(mn, md);
                if mat == smelt_core::config::Materialization::Ephemeral {
                    let t = config.get_target(mn, md, &request.target);
                    ephemerals_by_target_dry
                        .entry(t)
                        .or_default()
                        .push((mn.clone(), mf.content.clone()));
                }
            }
        }
        let mut ephemeral_resolvers_dry: HashMap<String, EphemeralResolver> = HashMap::new();
        for target_name in &needed_targets_dry {
            let schema = &config.targets[target_name].schema;
            let models_slice = ephemerals_by_target_dry
                .get(target_name)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let compiler = compilers_dry.get(target_name);
            ephemeral_resolvers_dry.insert(
                target_name.clone(),
                compiler.build_ephemeral_resolver(models_slice, schema)?,
            );
        }
        // Inject seed CTEs.
        if !request.ephemeral_seed_ctes.is_empty() {
            for resolver in ephemeral_resolvers_dry.values_mut() {
                resolver.add_seed_ctes(request.ephemeral_seed_ctes.clone());
            }
        }

        static EMPTY_RESOLVER: std::sync::OnceLock<EphemeralResolver> = std::sync::OnceLock::new();

        // T3 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
        // Phase E3): resolved once per dry-run, keyed by canonical
        // address, so the per-model loop below can build each model's
        // `ModelEdge` list without re-scanning every model in the
        // workspace per ref. Mirrors `propagation.rs::
        // derive_clamp_and_locality`'s own `model_by_addr` map.
        let model_by_addr_dry: HashMap<String, smelt_core::ModelFile> = graph_lock
            .iter_models()
            .map(|(_, m)| (m.canonical_path(), m.clone()))
            .collect();

        for model_name in selected {
            let Ok(model_file) = graph_lock.get_model(model_name) else {
                reporter.model_compiled(run_id, model_name, "");
                continue;
            };
            let metadata = model_file.metadata.as_deref();
            let mat = config.get_materialization_with_metadata(model_name, metadata);
            // Ephemerals are inlined — no standalone SQL to show.
            if mat == smelt_core::config::Materialization::Ephemeral {
                continue;
            }
            let model_target = config.get_target(model_name, metadata, &request.target);
            let schema = &config.targets[&model_target].schema;
            let compiler = compilers_dry.get(&model_target);
            let resolver = ephemeral_resolvers_dry
                .get(&model_target)
                .unwrap_or_else(|| EMPTY_RESOLVER.get_or_init(EphemeralResolver::empty));
            let clean_sql = smelt_parser::strip_frontmatter(&model_file.content);
            // The `model_compiled` display shows the unfiltered (full-refresh
            // form) SELECT, as it always has; the maintenance statements
            // below carry the per-batch clamped bodies.
            match compiler.compile_with_sql_and_ephemerals(model_file, schema, &clean_sql, resolver)
            {
                Ok(compiled) => reporter.model_compiled(run_id, model_name, &compiled.sql),
                Err(_) => {
                    reporter.model_compiled(run_id, model_name, "");
                    continue;
                }
            };

            // ── Maintenance statements this invocation would execute ────
            // Real window literals, one `StatementGroup` per chunk — the
            // output of the same single-owner emitters a real run consumes
            // (`docs/specs/incremental_models.md` §"Statement emission (single
            // owner)"), not just the compiled SELECT body. A `grain:
            // partition` creation trigger lowers to the region-recompute
            // (DELETE+INSERT) technique; the chunk windows are the SAME
            // `build_model_plans` decomposition a real run walks, and each
            // batch's body is clamped by the SAME `derive_batch_filtered_sql`
            // the live run uses — so `--dry-run` reflects exactly what the
            // invocation would execute (`docs/specs/cli.md` §"`--dry-run`
            // prints the maintenance statements"). No SQL is authored here;
            // the clamp is injected and handed to the emitter.
            let Some(plan) = model_plans.iter().find(|p| &p.name == model_name) else {
                continue;
            };
            let Some(inc) = plan.incremental.as_ref() else {
                continue;
            };
            let dialect = maintenance_dialect_for_target(config, &model_target);
            let partition_col = &inc.timeseries.partition_column;
            let table_name = format!("{schema}.{}", model_file.db_name_owned());
            let per_model_source_bounds =
                build_model_source_bounds(model_file, source_timeseries, model_name);
            // T3 (`docs/plans/20260715-composed-axes-conditional-
            // maintenance.md` Phase E3): resolved once per model (not
            // per batch — the facts don't vary across this model's own
            // batches). A dry-run has no backend to read the
            // observed-delta table from (`docs/specs/architecture.md`
            // §"Run pipeline parity rule" — dry-run never calls
            // `BackendFactory::create`), so the reported statement
            // below is always the ordinary widened scan (honest: a
            // dry-run cannot know whether a live run's delta read would
            // restrict) — but it is reached through the SAME dispatch
            // call (`build_delete_insert_group_dispatched`) the live
            // executor uses, never a hand-picked `emit_delete_insert`
            // call, so the two paths cannot structurally diverge.
            let model_edges_dry = model_edges_for(model_file, &model_by_addr_dry, source_infos);
            let delta_facts_dry = if model_edges_dry.is_empty() {
                None
            } else {
                match model_file.metadata.as_deref() {
                    Some(metadata) => {
                        let (sources, explicitly_mutable) =
                            build_maint_source_facts(model_file, source_infos);
                        crate::maintenance_driver::resolve_live_delta_restriction_facts(
                            &clean_sql,
                            &model_file.db_name_owned(),
                            metadata,
                            &sources,
                            &explicitly_mutable,
                            &model_edges_dry,
                            &availability_for_target(state_availability, &model_target, config),
                        )
                        .map_err(|e| anyhow::anyhow!("{}", e))?
                    }
                    None => None,
                }
            };
            let (restrict_column_dry, skeleton_source_closure_dry, region_write_dry) =
                match delta_facts_dry {
                    Some(facts) => (
                        facts.restrict_column,
                        facts.skeleton_source_closure,
                        Some(facts.region_write),
                    ),
                    None => (None, None, None),
                };
            for (batch_idx, batch) in inc.batches.iter().enumerate() {
                let start = batch.partition_start.to_string();
                let end = batch.partition_end.to_string();
                let run_range = TimeRange {
                    start: start.clone(),
                    end: end.clone(),
                    axis: batch.partition_start.axis(),
                };
                let filtered_sql = derive_batch_filtered_sql(
                    &clean_sql,
                    partition_col,
                    &per_model_source_bounds,
                    &run_range,
                    run_start,
                    inc.skew,
                )?;
                let compiled = compiler.compile_with_sql_and_ephemerals(
                    model_file,
                    schema,
                    &filtered_sql,
                    resolver,
                )?;
                let region = smelt_logical::maintenance::emit::Region::for_axis(
                    run_range.axis,
                    &start,
                    &end,
                )
                .map_err(anyhow::Error::msg)?;
                let group = crate::maintenance_driver::build_delete_insert_group_dispatched(
                    &table_name,
                    partition_col,
                    &region,
                    &compiled.sql,
                    restrict_column_dry.as_deref(),
                    skeleton_source_closure_dry.as_ref(),
                    None,
                    region_write_dry.as_ref(),
                    dialect,
                );
                let chunk = crate::reporter::ChunkInfo {
                    index: batch_idx,
                    total: inc.batches.len(),
                    start,
                    end,
                };
                reporter.maintenance_statements(run_id, model_name, Some(&chunk), &group);
            }
        }
    }

    Ok(RunOutcome {
        run_id: run_id.to_string(),
        started_at: run_start,
        completed_at: Some(Utc::now()),
        models: HashMap::new(),
        total_rows: 0,
        plan_summary: Some(plan_summary),
        check_results: vec![],
    })
}
