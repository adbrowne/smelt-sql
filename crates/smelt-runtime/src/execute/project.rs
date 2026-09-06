use super::backend::*;
use super::bootstrap::*;
use super::key_addressed::*;
use super::outcome::*;
use super::plan::*;
use super::retry::*;
use super::sink::*;
use super::sources::*;
use super::targets::*;
use super::window::*;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use futures::stream::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use smelt_backend::{Backend, Materialization, MaterializationStrategy, PartitionRange};
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_planner::Frontmatter;
use smelt_state::file_store::FileStore;
use smelt_state::intervals::compute_model_hash;
use smelt_state::landed_deltas::{record_landing, SourceMutationPosture};
use smelt_state::{ModelRunRecord, RunManifest, TimeRangeRecord};

use crate::check_runner::CheckOutcome;
use crate::compile::build_source_bound_map;
use crate::compile::CompilerRegistry;
use crate::reporter::RunReporter;
use crate::safety::{build_model_graph, check_bound_derivation, check_planner_safety};
use crate::schema_evolution::{
    check_and_migrate, ddl_backend_for_dialect, extract_evolution_maps,
    full_refresh_escape_requires_rebuild, infer_deployed_columns, SchemaEvolutionResult,
};
use crate::select::{select_executable_models, SelectionRequest};
use crate::transformer::TimeRange;
use crate::types::{ExecuteRequest, ModelPlanRecord, ModelStrategy, PlanSummary, RunOutcome};
use crate::{build_fn_body_map, EphemeralResolver, UpstreamSchemas};

/// Run the project end-to-end.
///
/// `graph` and `db` are passed as ref-counted async mutexes so the runtime
/// can do brief reads (build `UpstreamSchemas`, enumerate ephemerals) and
/// release the locks before the long execution phase. The graph lock is
/// released before any backend call; the DB lock is released after compile
/// context is built.
///
/// `reporter` receives progress callbacks in this order on a successful run:
/// `run_started → (model_started → batch_completed* → model_completed)+ →
/// run_completed`. A cancelled run sends `run_cancelled`; a failed run sends
/// `run_failed` with the failing model.
#[allow(clippy::too_many_arguments)]
pub async fn execute_project(
    run_id: String,
    request: ExecuteRequest,
    config: Arc<Config>,
    graph: Arc<tokio::sync::Mutex<DependencyGraph>>,
    db: Arc<tokio::sync::Mutex<smelt_db::Database>>,
    project_dir: &Path,
    backend_factory: &dyn BackendFactory,
    reporter: &dyn RunReporter,
    cancel: CancellationToken,
) -> Result<RunOutcome> {
    let run_start = Utc::now();
    let execution_start = Instant::now();

    if !config.targets.contains_key(&request.target) {
        anyhow::bail!("Target '{}' not found", request.target);
    }

    // ── Selection ───────────────────────────────────────────────────────
    let graph_lock = graph.lock().await;

    let selection_request = SelectionRequest {
        select: request.select.clone(),
        exclude: request.exclude.clone(),
        target: request.target.clone(),
    };
    let selection = select_executable_models(&graph_lock, &config, &selection_request)?;
    let selected = selection.ordered_models;
    let target_assignments = selection.target_assignments;
    let cross_edges = selection.cross_engine_edges;

    // The run's availability-resolution input (`docs/specs/state.md` §"The
    // degradation contract" step 2): one `StateAvailability` per target this
    // run touches, built once here (never rebuilt per model) from the
    // target's declared dialect and the project-wide `state.warehouse_tables`
    // — both pure `Config` facts, so this needs no live backend and is
    // reused by the dry-run statement preview, the pre-loop deferral pass,
    // and the real per-model execute loop alike.
    let state_availability: HashMap<
        String,
        smelt_logical::maintenance::availability::StateAvailability,
    > = target_assignments
        .values()
        .cloned()
        .collect::<HashSet<String>>()
        .into_iter()
        .map(|target| {
            let dialect = sql_dialect_for_target(&config, &target);
            let availability =
                crate::maintenance_availability::availability_for_run(dialect, &config);
            (target, availability)
        })
        .collect();
    if !cross_edges.is_empty() {
        tracing::info!(
            "Cross-engine references detected ({} transfer(s) via Parquet)",
            cross_edges.len()
        );
    }

    // ── Function bodies (needed for both dry_run and live run) ──────────────
    let fn_bodies = {
        let db_guard = db.lock().await;
        let db_ref: &smelt_db::Database = &db_guard;
        let workspace = smelt_db::Workspace::try_get(db_ref)
            .ok_or_else(|| anyhow::anyhow!("workspace not initialised in DB"))?;
        build_fn_body_map(db_ref, workspace)
    };

    // ── Diagnostic-parity gate (analysis ↔ build) — runs for dry_run too ────
    {
        let mut model_paths: Vec<std::path::PathBuf> = selected
            .iter()
            .filter_map(|name| {
                graph_lock.get_model(name).ok().map(|m| {
                    // Python-emitted models are stored in the Salsa DB under a virtual path
                    // (e.g. `py_source.py::py_source`) so parse_model derives the correct name.
                    // But the DependencyGraph retains the original `.py` path.  Recompute the
                    // virtual path here so gate_diagnostics can look them up.
                    let path = m.path.clone();
                    if path.to_string_lossy().ends_with(".py") {
                        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("py");
                        path.with_file_name(format!("{}::{}", filename, m.name))
                    } else {
                        path
                    }
                })
            })
            .collect();
        model_paths.extend(smelt_core::discover_function_file_paths(project_dir));
        let db_guard = db.lock().await;
        let db_ref: &smelt_db::Database = &db_guard;
        let workspace = smelt_db::Workspace::try_get(db_ref)
            .ok_or_else(|| anyhow::anyhow!("workspace not initialised in DB"))?;
        if let Err(errors) = crate::gate::gate_diagnostics(db_ref, workspace, &model_paths) {
            anyhow::bail!("{}", crate::gate::format_gate_errors(&errors));
        }
    }

    // ── Planner safety check + temporal bound derivation (also for dry_run) ──
    // Must run before the dry_run early return so that `--dry-run` on an
    // unsafe incremental model (bare LAG, NotDerivable bound) is refused
    // identically to a live run. This mirrors the CLI's original behaviour
    // where `--dry-run` runs through compilation and thus through the planner.
    {
        let model_graph = build_model_graph(&selected, &graph_lock, &config, &fn_bodies);
        check_planner_safety(&model_graph, request.enforce_safety)?;
        check_bound_derivation(&model_graph, request.enforce_safety)?;
    }

    // ── Time range + source discovery + model-plan (chunk) construction ─────
    // Built before the dry-run early return because both the dry-run
    // statement-emission branch and the real run consume the identical chunk
    // decomposition and window literals — a `--dry-run` must show exactly the
    // chunks (and their `[start, end)` windows) a real run would execute
    // (`docs/specs/cli.md` §"`--dry-run` prints the maintenance statements").
    // None of these touch a backend.
    let (start_date, end_date) = parse_run_window(&request)?;

    // Project-wide `smelt.<path> → timeseries` map. Merges model-frontmatter
    // timeseries with per-entity source YAML timeseries declarations (BUG-072).
    // Consumed by the model-plan construction below (BL2's bound-based
    // batch-safety derivation) and, further down, by keyed dispatch and
    // incremental pushdown.
    let source_infos = smelt_core::discover_source_infos(project_dir, &config.paths);
    let source_timeseries = build_source_timeseries_map(&graph_lock, &source_infos);
    let source_key_recurrence = build_source_key_recurrence_map(&source_infos);

    // Each selected model's `partition_column` axis (`docs/specs/timeseries.md`
    // §"Validation rules" rule 9), resolved from its output schema — reuses
    // the same `resolved_model_schema` Salsa read `UpstreamSchemas::from_database`
    // performs, rather than building a second `UpstreamSchemas`. A model
    // missing from this map (unresolvable type) falls back inside
    // `build_model_plans` to the axis implied by the run-window literal's
    // own form.
    let partition_axes = {
        let db_guard = db.lock().await;
        let db_ref: &smelt_db::Database = &db_guard;
        resolve_partition_axes(db_ref, &selected, &graph_lock, &config)
    };

    let (model_plans, total_batches) = build_model_plans(
        &selected,
        &graph_lock,
        &config,
        &fn_bodies,
        &source_timeseries,
        start_date,
        end_date,
        &request,
        &partition_axes,
    )?;

    // ── Dry-run: build PlanSummary, compile models, and return without any backend call ──────
    // When `dry_run = true` we resolve the execution strategy per model from
    // graph config alone, compile each model's SQL (full-refresh form only),
    // emit `reporter.model_compiled` for each, and return the plan without
    // invoking `BackendFactory::create` or executing any SQL.
    //
    // Parity rule: the reporter callback is the only place compiled SQL is
    // surfaced to the consumer. CLI/UI must not re-implement compilation after
    // this returns. See `docs/specs/architecture.md` §"Run pipeline parity rule".
    if request.dry_run {
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
                            smelt_core::config::Materialization::Ephemeral => {
                                ModelStrategy::Ephemeral
                            }
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

            let mut compilers_dry =
                CompilerRegistry::new(config.as_ref(), &needed_target_configs_dry);

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
                &source_timeseries,
            ));
            if !fn_bodies.is_empty() {
                compilers_dry.set_function_bodies_all(fn_bodies);
            }

            // Build ephemeral resolvers.
            let mut ephemerals_by_target_dry: HashMap<String, Vec<(String, String)>> =
                HashMap::new();
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

            static EMPTY_RESOLVER: std::sync::OnceLock<EphemeralResolver> =
                std::sync::OnceLock::new();

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

            for model_name in &selected {
                let Ok(model_file) = graph_lock.get_model(model_name) else {
                    reporter.model_compiled(&run_id, model_name, "");
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
                match compiler
                    .compile_with_sql_and_ephemerals(model_file, schema, &clean_sql, resolver)
                {
                    Ok(compiled) => reporter.model_compiled(&run_id, model_name, &compiled.sql),
                    Err(_) => {
                        reporter.model_compiled(&run_id, model_name, "");
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
                let dialect = maintenance_dialect_for_target(&config, &model_target);
                let partition_col = &inc.timeseries.partition_column;
                let table_name = format!("{schema}.{}", model_file.db_name_owned());
                let per_model_source_bounds =
                    build_model_source_bounds(model_file, &source_timeseries, model_name);
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
                let model_edges_dry =
                    model_edges_for(model_file, &model_by_addr_dry, &source_infos);
                let delta_facts_dry = if model_edges_dry.is_empty() {
                    None
                } else {
                    match model_file.metadata.as_deref() {
                        Some(metadata) => {
                            let (sources, explicitly_mutable) =
                                build_maint_source_facts(model_file, &source_infos);
                            crate::maintenance_driver::resolve_live_delta_restriction_facts(
                                &clean_sql,
                                &model_file.db_name_owned(),
                                metadata,
                                &sources,
                                &explicitly_mutable,
                                &model_edges_dry,
                                &availability_for_target(
                                    &state_availability,
                                    &model_target,
                                    &config,
                                ),
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
                    reporter.maintenance_statements(&run_id, model_name, Some(&chunk), &group);
                }
            }
        }

        let outcome = RunOutcome {
            run_id: run_id.to_string(),
            started_at: run_start,
            completed_at: Some(Utc::now()),
            models: HashMap::new(),
            total_rows: 0,
            plan_summary: Some(plan_summary),
            check_results: vec![],
        };
        drop(graph_lock);
        return Ok(outcome);
    }

    // ── Backend creation ────────────────────────────────────────────────
    let needed_targets: HashSet<String> = target_assignments.values().cloned().collect();
    let mut backends: HashMap<String, Box<dyn Backend>> = HashMap::new();
    for target_name in &needed_targets {
        let target_config = config
            .targets
            .get(target_name)
            .ok_or_else(|| anyhow::anyhow!("Target '{}' not found", target_name))?;
        let backend = backend_factory
            .create(target_name, target_config, project_dir)
            .await?;
        backends.insert(target_name.clone(), backend);
    }

    // `start_date`/`end_date`, `source_infos`/`source_timeseries`, and
    // `model_plans`/`total_batches` are all built before the dry-run early
    // return (see the block just above the dry-run branch) so the two paths
    // share one chunk decomposition; the real run below consumes them directly.

    let all_models: Vec<smelt_core::ModelFile> =
        graph_lock.iter_models().map(|(_, m)| m.clone()).collect();
    // T3 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    // Phase E3): keyed by canonical address so the real per-batch loop
    // below can build each model's `ModelEdge` list (`model_edges_for`)
    // without re-scanning every model per ref.
    let model_by_addr: HashMap<String, smelt_core::ModelFile> = all_models
        .iter()
        .map(|m| (m.canonical_path(), m.clone()))
        .collect();
    let mut ephemeral_models_by_target: HashMap<String, Vec<(String, String)>> = HashMap::new();
    {
        let exec_order = graph_lock.execution_order()?;
        for model_name in &exec_order {
            let Ok(model) = graph_lock.get_model(model_name) else {
                continue;
            };
            let metadata = model.metadata.as_deref();
            let mat = config.get_materialization_with_metadata(model_name, metadata);
            if mat == smelt_core::config::Materialization::Ephemeral {
                let target = config.get_target(model_name, metadata, &request.target);
                ephemeral_models_by_target
                    .entry(target)
                    .or_default()
                    .push((model_name.clone(), model.content.clone()));
            }
        }
    }
    // `source_infos`/`source_timeseries` are built earlier (before model-plan
    // construction) — see the comment there.
    drop(graph_lock);

    // ── Compile context (UpstreamSchemas + FnBodyMap from Salsa) ────────
    let needed_target_configs: HashMap<String, smelt_core::config::Target> = needed_targets
        .iter()
        .filter_map(|t| config.targets.get(t).map(|c| (t.clone(), c.clone())))
        .collect();
    let mut compilers = CompilerRegistry::new(config.as_ref(), &needed_target_configs);

    let upstream_schemas = {
        let db_guard = db.lock().await;
        let db_ref: &smelt_db::Database = &db_guard;
        let upstream = UpstreamSchemas::from_database(db_ref, project_dir, &all_models)?;
        Arc::new(upstream)
    };
    // Kept alongside the registry's own clone (`set_upstream_schemas_all`
    // moves its argument) — the self-referential first-run bootstrap below
    // reads `upstream_schemas.models` directly for the SAME resolved output
    // schema `apply_type_casts` already uses for every other model.
    let upstream_schemas_for_bootstrap = Arc::clone(&upstream_schemas);
    compilers.set_upstream_schemas_all(upstream_schemas);
    compilers
        .set_state_bearing_models_all(build_state_bearing_models(&all_models, &source_timeseries));
    if !fn_bodies.is_empty() {
        compilers.set_function_bodies_all(fn_bodies);
    }

    // ── Cross-engine Parquet reference wiring ────────────────────────────
    // For each cross-engine edge (consumer_model, producer_dep, consumer_target,
    // producer_target), ask the producer backend for its materialized filesystem
    // path and inject a read_parquet(...) substitution into the consumer target's
    // compiler so smelt.ref(dep) resolves to read_parquet instead of a table name.
    // Spec oracle: multi_backend.md §"Cross-engine data exchange".
    if !cross_edges.is_empty() {
        let mut refs_by_target: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (_consumer, dep_name, consumer_target, producer_target) in &cross_edges {
            if let Some(producer) = backends.get(producer_target.as_str()) {
                let dep_schema = config
                    .targets
                    .get(producer_target)
                    .map(|t| t.schema.as_str())
                    .unwrap_or("");
                // Compiler looks up cross-engine refs by the underscore form of the dep path
                // (segs.join("_"), matching how smelt.<path> refs are resolved in compile.rs).
                let dep_db_name = dep_name.replace('.', "_");
                if let Some(path) = producer.materialized_path(dep_schema, &dep_db_name) {
                    let parquet_expr = format!(
                        "read_parquet('{}/**/*.parquet', hive_partitioning = true)",
                        path.display()
                    );
                    refs_by_target
                        .entry(consumer_target.clone())
                        .or_default()
                        .insert(dep_db_name, parquet_expr);
                }
            }
        }
        for (target_name, refs) in refs_by_target {
            compilers.set_cross_engine_refs(&target_name, refs);
        }
    }

    let mut ephemeral_resolvers: HashMap<String, EphemeralResolver> = HashMap::new();
    for target_name in &needed_targets {
        let schema = &config.targets[target_name].schema;
        let models = ephemeral_models_by_target
            .get(target_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let compiler = compilers.get(target_name);
        ephemeral_resolvers.insert(
            target_name.clone(),
            compiler.build_ephemeral_resolver(models, schema)?,
        );
    }

    // Inject pre-built ephemeral seed CTEs from the request (e.g. CSV seeds
    // with `materialization: ephemeral` discovered by the CLI). The CTEs are
    // already formatted (alias_with_cols + VALUES body) so they bypass the
    // SQL compiler and are added directly to every resolver.
    if !request.ephemeral_seed_ctes.is_empty() {
        for resolver in ephemeral_resolvers.values_mut() {
            resolver.add_seed_ctes(request.ephemeral_seed_ctes.clone());
        }
    }

    // ── Check infrastructure (build integration) ──────────────────────────
    // Pre-compute all data needed for check execution before the model loop:
    // - checks_by_model: model name → check ModelFiles that reference it
    // - upstream_map: selected model name → transitive upstream set
    //   (used to find "downstream of X" by inverting: m is downstream of X
    //   iff upstream_map[m].contains(X))
    let checks_by_model: HashMap<String, Vec<smelt_core::ModelFile>> = if request.run_checks {
        let mut map: HashMap<String, Vec<smelt_core::ModelFile>> = HashMap::new();
        for check_model in &request.checks {
            // A check references a model when it has a smelt.<path> ref whose
            // joined segments match a selected model name.
            for ref_info in &check_model.refs {
                let segs = ref_info.smelt_ref.to_path();
                if segs.is_empty() || segs[0] == "sources" || segs[0] == "functions" {
                    continue;
                }
                let model_name = segs.join(".");
                if selected.contains(&model_name) {
                    map.entry(model_name.clone())
                        .or_default()
                        .push(check_model.clone());
                }
            }
        }
        map
    } else {
        HashMap::new()
    };

    // The run pipeline's store is posture-gated (`docs/specs/state.md`
    // §"`state.mode` and what each posture provides"): under `stateless`
    // every save/load below is a no-op and `.smelt/` is never created.
    let file_store = FileStore::with_state_mode(project_dir, &request.target, config.state.mode);

    // Legacy `sources.yml` — best-effort input to the definition-delta gate's
    // upstream-facts map (`docs/specs/definition_deltas.md` §"The migration
    // plan"), same posture as `smelt migrate`'s own load: a missing or
    // unparseable file just means fewer admitted techniques, never a
    // hard failure.
    let legacy_sources_config = smelt_core::sources::SourcesConfig::load(project_dir).ok();

    // Models declaring `contract.deferral` at model granularity, or
    // `contract.cells[].deferral` at cell granularity — the latter is only
    // ever validly declared on a plain `Trigger::NewData` fold cell over a
    // clocked source (`docs/outcomes/20260815-definition-delta-migrate/
    // phases/14-plan.md`); this set's only job here is widening the
    // `upstream_map` build condition and the pre-run snapshot loop below,
    // since a deferral skip (model- or cell-level) must also propagate to
    // dependents (`docs/specs/incremental_models.md` §"The contract
    // lattice").
    let deferral_declared: HashSet<String> = model_plans
        .iter()
        .filter(|p| {
            p.model_file
                .metadata
                .as_deref()
                .and_then(|m| m.contract.as_ref())
                .is_some_and(|c| {
                    c.deferral.is_some() || c.cells.iter().any(|c| c.deferral.is_some())
                })
        })
        .map(|p| p.name.clone())
        .collect();

    // Build model → all_upstream map for the selected set (needed for
    // downstream closure computation — both the check-skip downstream
    // closure, the `--resume` downstream closure, and the deferral-skip
    // downstream closure below). Captured here to avoid holding the graph
    // lock across awaits in the model loop.
    let upstream_map: HashMap<String, HashSet<String>> =
        if request.run_checks || request.resume || !deferral_declared.is_empty() {
            // graph_lock was already dropped above; we need to re-lock briefly to
            // read all_upstream for each selected model.
            // Actually, graph was dropped before backends were created.
            // We need to rebuild from the model graph. Since we dropped graph_lock,
            // we captured needed data already. But we need all_upstream.
            // Use the already-built model_plans to reconstruct deps from model_file refs.
            // Actually, model_file.refs captures the smelt refs, not the canonical names.
            // The cleanest: re-lock the graph briefly just to capture upstream maps.
            // This is safe because graph is only mutated before the lock is dropped.
            let graph_lock2 = graph.lock().await;
            let map: HashMap<String, HashSet<String>> = selected
                .iter()
                .map(|name| (name.clone(), graph_lock2.all_upstream(name)))
                .collect();
            drop(graph_lock2);
            map
        } else {
            HashMap::new()
        };

    let mut skip_set: HashSet<String> = HashSet::new();
    let mut check_results: Vec<CheckOutcome> = Vec::new();

    // ── Execute loop ────────────────────────────────────────────────────
    reporter.run_started(
        &run_id,
        &model_plans
            .iter()
            .map(|m| m.name.clone())
            .collect::<Vec<_>>(),
        total_batches,
    );

    // ── `--resume`: locate the run to resume from, fail loud if there is
    // none ────────────────────────────────────────────────────────────────
    // `docs/specs/run_state.md` §"`--resume` semantics": the candidate is
    // the latest manifest for this target with `completed_at: null` (an
    // incomplete run), OR — since `load_runs` is newest-first — the latest
    // manifest overall whose model selection overlaps the current one and
    // that recorded at least one non-`success` outcome (a run that
    // completed the wavefront scheduler but left some models `skipped`,
    // e.g. because a check failure skipped their downstream dependents
    // without aborting the whole run). `--resume` with no such run (the
    // latest run succeeded cleanly, or no manifest exists at all) is a hard
    // error, never a silent full run, so a typo'd `--resume` on a clean
    // project can't be mistaken for "nothing needed doing".
    let current_selection: HashSet<&str> = model_plans.iter().map(|p| p.name.as_str()).collect();
    let resume_manifest: Option<RunManifest> = if request.resume {
        // `docs/specs/state.md` §"The optionality rule": `stateless` has no
        // manifest to resume from *by posture*, not by accident — refuse by
        // name rather than falling through to the generic "no partially-
        // failed run" message a missing manifest would otherwise produce.
        if config.state.mode == smelt_core::config::StateMode::Stateless {
            anyhow::bail!(
                "--resume: target '{}' is running with state.mode: stateless, which writes no \
                 run history — there is nothing to resume from. Set state.mode to intervals or \
                 environments to enable --resume.",
                request.target
            );
        }
        let latest_candidate = file_store
            .load_runs(None)
            .context("--resume: failed to load run history")?
            .into_iter()
            .find(|m| {
                m.completed_at.is_none()
                    || (m
                        .models
                        .keys()
                        .any(|name| current_selection.contains(name.as_str()))
                        && m.models
                            .values()
                            .any(|rec| rec.outcome != smelt_state::RunOutcomeKind::Success))
            });
        Some(latest_candidate.ok_or_else(|| {
            anyhow::anyhow!(
                "--resume: no partially-failed run found for target '{}' — the most recent \
                 run (if any) completed successfully, so there is nothing to resume. Run \
                 without --resume, or remove .smelt/ to start fresh.",
                request.target
            )
        })?)
    } else {
        None
    };

    // Models this run skips because `--resume` found them already
    // `success` last time with an unchanged `definition_hash`. A model
    // whose prior outcome was `failed`/`skipped`, whose definition changed,
    // or that is downstream of any such model always re-runs — computed as
    // a fixpoint over `upstream_map` (built above whenever `request.resume`
    // is set) so a re-run never leaves a stale downstream table in place
    // (`docs/specs/run_state.md` §"`--resume` semantics").
    let resume_skip_set: HashSet<String> = if let Some(ref prior) = resume_manifest {
        let mut rerun: HashSet<String> = HashSet::new();
        for plan in model_plans.iter() {
            let cur_hash = compute_model_hash(&plan.sql);
            let stays_skipped = prior.models.get(&plan.name).is_some_and(|rec| {
                rec.outcome == smelt_state::RunOutcomeKind::Success
                    && rec.definition_hash == cur_hash
            });
            if !stays_skipped {
                rerun.insert(plan.name.clone());
            }
        }
        loop {
            let mut added = false;
            for plan in model_plans.iter() {
                if rerun.contains(&plan.name) {
                    continue;
                }
                if let Some(ups) = upstream_map.get(&plan.name) {
                    if ups.iter().any(|u| rerun.contains(u)) {
                        rerun.insert(plan.name.clone());
                        added = true;
                    }
                }
            }
            if !added {
                break;
            }
        }
        model_plans
            .iter()
            .map(|p| p.name.clone())
            .filter(|n| !rerun.contains(n))
            .collect()
    } else {
        HashSet::new()
    };

    // ── `contract.deferral`: the run-skip license and pending window for
    // every model declaring it, computed once before the wavefront
    // scheduler runs (`docs/outcomes/20260809-contract-lattice-v1/phases/
    // 05-plan.md`) — both ledger frontiers this reads are untouched until
    // this run's own writes touch them, so one upfront snapshot is correct
    // for every model's decision. `deferral_own_skip` is the set licensed
    // to skip on its own declaration; `deferral_pending` is every declaring
    // model's pending window (`None` when nothing is pending), consulted
    // later to prove work subsumption on a covering run.
    // Named so the `let` below stays under clippy's `type_complexity` gate —
    // both maps are per-model cell-address lists, one for the addresses a
    // `SkipFold` verdict named, one for every declaring address a model
    // that reached the fold-deferral branch owns (see the field's own doc
    // comment a few lines down).
    type PerModelCellAddresses = HashMap<String, Vec<String>>;
    let (deferral_own_skip, deferral_pending, deferral_skipped_cells, deferral_fold_addresses): (
        HashSet<String>,
        HashMap<String, smelt_logical::contract::deferral::PendingWindow>,
        PerModelCellAddresses,
        PerModelCellAddresses,
    ) = if deferral_declared.is_empty() {
        (
            HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
    } else {
        let interval_store = file_store.load_intervals().unwrap_or_default();
        let landed_deltas = file_store.load_landed_deltas().unwrap_or_default();
        let mut own_skip = HashSet::new();
        let mut pending = HashMap::new();
        let mut skipped_cells: HashMap<String, Vec<String>> = HashMap::new();
        // Every declaring `contract.cells[].deferral` address for a model
        // that reaches the fold-deferral branch below, independent of the
        // resolved verdict — a fold that actually runs (`Proceed`, or a
        // later catch-up run past a prior skip) advances ALL of its own
        // declaring cells' frontiers, since the plain fold's write is
        // whole-row (`incremental_models.md` §Known Divergences: "its
        // frontier advances with the rest"). Consulted from the incremental
        // success path below, not re-derived per model.
        let mut fold_addresses: HashMap<String, Vec<String>> = HashMap::new();
        for plan in model_plans.iter() {
            if !deferral_declared.contains(&plan.name) {
                continue;
            }
            let (source_facts, explicitly_mutable) =
                build_maint_source_facts(&plan.model_file, &source_infos);
            let clocked_source_addresses: Vec<String> = source_facts
                .iter()
                .filter(|sf| {
                    sf.partition_col.is_some()
                        && sf.mutation == smelt_logical::maintenance::MutationProfile::AppendOnly
                })
                .map(|sf| sf.name.clone())
                .collect();
            if let Some(decision) = crate::contract_probes::deferral_decision(
                &plan.name,
                plan.model_file.metadata.as_deref(),
                &clocked_source_addresses,
                &interval_store,
                &landed_deltas,
            ) {
                if let smelt_logical::contract::deferral::RunLicense::Skip { lag, d } =
                    decision.license
                {
                    tracing::info!(
                        "Deferring model '{}' — measured lag ({} day(s)) is within the \
                         declared deferral window (D={} day(s))",
                        plan.name,
                        lag,
                        d
                    );
                    own_skip.insert(plan.name.clone());
                }
                if let Some(window) = decision.pending {
                    pending.insert(plan.name.clone(), window);
                }
            }

            // `contract.cells[].deferral` on the plain `Trigger::NewData`
            // fold — the per-cell counterpart of the model-level decision
            // above, licensed independently (`docs/outcomes/
            // 20260815-definition-delta-migrate/phases/14-plan.md`). A model
            // already licensed to skip at model granularity has nothing more
            // to resolve here.
            if own_skip.contains(&plan.name) {
                continue;
            }
            if let Some(metadata) = plan.model_file.metadata.as_deref() {
                if metadata
                    .contract
                    .as_ref()
                    .is_some_and(|c| c.cells.iter().any(|cell| cell.deferral.is_some()))
                {
                    let cell_decisions = crate::contract_probes::deferral_cell_decisions(
                        &plan.name,
                        Some(metadata),
                        &interval_store,
                        &landed_deltas,
                    );
                    let declared_addresses: Vec<String> = metadata
                        .contract
                        .as_ref()
                        .map(|c| {
                            c.cells
                                .iter()
                                .filter(|cell| cell.deferral.is_some())
                                .map(|cell| {
                                    smelt_logical::contract::deferral::cell_address(
                                        &cell.columns,
                                        &cell.on,
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if !declared_addresses.is_empty() {
                        fold_addresses.insert(plan.name.clone(), declared_addresses);
                    }
                    let plan_target =
                        config.get_target(&plan.name, Some(metadata), &request.target);
                    let (verdict, addresses) = crate::maintenance_driver::resolve_fold_deferral(
                        &plan.sql,
                        &plan.model_file.db_name_owned(),
                        metadata,
                        &source_facts,
                        &explicitly_mutable,
                        &cell_decisions,
                        &availability_for_target(&state_availability, &plan_target, &config),
                    );
                    if let smelt_logical::contract::deferral::FoldDeferralVerdict::SkipFold {
                        ..
                    } = verdict
                    {
                        tracing::info!(
                            "Deferring model '{}''s plain incremental fold — every column \
                             group is covered by a skip-licensed contract.cells[].deferral \
                             declaration ({:?})",
                            plan.name,
                            addresses
                        );
                        own_skip.insert(plan.name.clone());
                        skipped_cells.insert(plan.name.clone(), addresses);
                    }
                }
            }
        }
        (own_skip, pending, skipped_cells, fold_addresses)
    };

    // A deferral skip propagates to dependents (`docs/outcomes/
    // 20260809-contract-lattice-v1/outcome.md` phase 5 decision log) — see
    // `contract_probes::propagate_deferral_skip`'s doc comment.
    let deferral_skip_set: HashSet<String> = crate::contract_probes::propagate_deferral_skip(
        &deferral_own_skip,
        &upstream_map,
        &model_plans
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>(),
    );

    // Hold the exclusive advisory lock on `.smelt/lock` for the remainder of
    // this run — every state write below (manifest, intervals,
    // reconciliation ledger, landed deltas, schema snapshots) happens while
    // `_state_lock` is alive. Dropping it (on success or any error/early
    // return from this point on, since it is an ordinary local binding)
    // releases the lock (`docs/specs/run_state.md` §"Locking").
    let _state_lock = file_store
        .lock()
        .context("failed to acquire the .smelt/ state lock")?;
    let mut manifest = RunManifest {
        run_id: run_id.clone(),
        started_at: run_start,
        completed_at: None,
        models: HashMap::new(),
    };

    let mut total_rows_overall: usize = 0;
    let models_total = model_plans.len();

    // Serializes the shared, whole-store-per-write `file_store` sections
    // (interval store / landed-delta store / reconciliation ledger — each a
    // single JSON blob covering every model) across concurrently executing
    // models. Without this, two models completing in the same wave could
    // each load-modify-save the SAME file and lose one write (a
    // load-modify-save race, not covered by `_state_lock` — that lock is
    // single-writer-per-*process*, not single-writer-per-in-process-task).
    // The backend call itself is NOT covered by this lock — only the cheap
    // local file I/O is, so it does not serialize the expensive part of a
    // model's work.
    let state_io_lock = tokio::sync::Mutex::new(());

    // Prior-run history for this target, loaded once per run and shared by
    // every model's `ProbePolicy` (`docs/specs/model_properties.md`
    // §"Probe cadence"): a model's run ordinal — 0 for its first run — is
    // its prior-run count via `HistoryQuery::for_model`.
    let prior_runs = file_store.load_runs(None).unwrap_or_default();

    // Every non-`Copy` piece of context the per-model execution unit below
    // needs is rebound here as a reference so the `move` closure — called
    // once per model, potentially many times concurrently in flight — can
    // capture each one BY VALUE as a cheap `Copy` reference instead of
    // moving the underlying owned data (which would make the closure
    // `FnOnce`, callable only once). The per-model body keeps referring to
    // these under their original names (`backends`, `config`, `run_id`,
    // …) unmodified — only this rebinding needed to change.
    let cancel = &cancel;
    let request = &request;
    let run_id = run_id.as_str();
    let checks_by_model = &checks_by_model;
    let upstream_map = &upstream_map;
    let selected = &selected;
    let file_store = &file_store;
    let config = &config;
    let state_availability = &state_availability;
    let prior_runs = &prior_runs;
    let target_assignments = &target_assignments;
    let backends = &backends;
    let compilers = &compilers;
    let ephemeral_resolvers = &ephemeral_resolvers;
    let source_infos = &source_infos;
    let model_by_addr = &model_by_addr;
    let source_timeseries = &source_timeseries;
    let source_key_recurrence = &source_key_recurrence;
    let upstream_schemas_for_bootstrap = &upstream_schemas_for_bootstrap;
    let db = &db;
    let legacy_sources_config = &legacy_sources_config;
    let all_models = &all_models;
    let state_io_lock = &state_io_lock;
    let model_plans = &model_plans;
    let resume_skip_set = &resume_skip_set;
    let deferral_own_skip = &deferral_own_skip;
    let deferral_skip_set = &deferral_skip_set;
    let deferral_pending = &deferral_pending;
    let deferral_skipped_cells = &deferral_skipped_cells;
    let deferral_fold_addresses = &deferral_fold_addresses;

    // ── Per-model execution unit ──────────────────────────────────────────
    // Runs one model to completion (or cancellation, or failure) and returns
    // its outcome instead of mutating shared run-level state directly —
    // the wavefront scheduler below runs many of these concurrently (bounded
    // by `request.jobs`) and merges outcomes back into `manifest`/
    // `check_results`/`skip_set`/`total_rows_overall` strictly in
    // `execution_order` sequence, one model at a time, so a concurrent
    // scheduling never produces a nondeterministic manifest or reporter
    // stream (`docs/plans/20260719-prod-w2-operability.md` Phase 5).
    //
    // `already_skip` is a snapshot of the run-level `skip_set` taken when
    // this model's wave started — safe because a wave's members share no
    // dependency edge with each other, and `skip_set` only ever gains
    // entries for a check's *downstream* models (`upstream_map`-derived),
    // so no same-wave sibling can skip-mark another.
    let execute_one_model = move |model_idx: usize, already_skip: bool| {
        let plan = &model_plans[model_idx];
        async move {
            // `sink` buffers every reporter callback this model's execution
            // produces; the scheduler replays them onto the real `reporter` only
            // once this model's turn comes up in `execution_order` sequence.
            // Shadowing the outer `reporter` binding means the (unmodified)
            // per-model logic below — inherited from the pre-Phase-5 sequential
            // loop — calls `sink` under the name `reporter` with no further
            // rewriting needed. The block's own `?` operator now short-circuits
            // just THIS model's execution (returning `Err` from this async
            // block) rather than the whole run — the scheduler treats any `Err`
            // the same way the pre-Phase-5 loop treated a `return Err(e)`: this
            // model failed, attributed to its own name.
            let sink = EventSink::default();
            let reporter: &dyn RunReporter = &sink;
            let mut manifest_entries: HashMap<String, ModelRunRecord> = HashMap::new();
            let mut check_results: Vec<CheckOutcome> = Vec::new();
            let mut skip_set: HashSet<String> = HashSet::new();
            let mut total_rows_overall: usize = 0;

            let outcome: Result<ModelOutcome> = async {
        if cancel.is_cancelled() {
            return Ok(ModelOutcome::Cancelled);
        }

        // ── `--resume`: skip a model that already succeeded last time with
        // an unchanged definition ─────────────────────────────────────────
        // No compilation, no backend call, no interval/reconciliation/
        // landed-delta write — a resumed-away model's materialized state
        // (and its interval-ledger bookkeeping) is left byte-for-byte
        // untouched (`docs/specs/run_state.md` §"`--resume` semantics").
        if resume_skip_set.contains(&plan.name) {
            tracing::info!(
                "Skipping model '{}' — succeeded in the run being resumed, definition unchanged",
                plan.name
            );
            manifest_entries.insert(
                plan.name.clone(),
                smelt_state::ModelRunRecord {
                    strategy: "skipped_resume".to_string(),
                    time_range: None,
                    partitions_updated: vec![],
                    row_count: 0,
                    duration_ms: 0,
                    batch_safety: Some("skipped".to_string()),
                    outcome: smelt_state::RunOutcomeKind::Skipped,
                    definition_hash: compute_model_hash(&plan.sql),
                    error: None,
                    retry_count: 0,
                    probes: Vec::new(),
                    subsumed: None,
                    deferred_cells: Vec::new(),
                },
            );
            reporter.model_completed(run_id, &plan.name, 0, std::time::Duration::ZERO);
            return Ok(ModelOutcome::Completed(ModelSuccess {
                manifest_entries,
                check_results,
                skip_set,
                rows: 0,
            }));
        }

        // ── `contract.deferral`: skip a model whose measured lag is a
        // licensed relaxation, or a dependent of one that was
        // (`docs/specs/incremental_models.md` §"The contract lattice") ────
        // No compilation, no backend call, no interval/reconciliation/
        // landed-delta write — recorded, never silently dropped, so a later
        // covering run can prove work subsumption from this very record.
        if deferral_skip_set.contains(&plan.name) {
            let own = deferral_own_skip.contains(&plan.name);
            let strategy = if own {
                "skipped_deferral"
            } else {
                "skipped_deferral_upstream"
            };
            tracing::info!(
                "Skipping model '{}' — {}",
                plan.name,
                if own {
                    "measured lag is within the declared deferral window"
                } else {
                    "downstream of a deferral-skipped model"
                }
            );
            manifest_entries.insert(
                plan.name.clone(),
                smelt_state::ModelRunRecord {
                    strategy: strategy.to_string(),
                    time_range: None,
                    partitions_updated: vec![],
                    row_count: 0,
                    duration_ms: 0,
                    batch_safety: Some("skipped".to_string()),
                    outcome: smelt_state::RunOutcomeKind::Skipped,
                    definition_hash: compute_model_hash(&plan.sql),
                    error: None,
                    retry_count: 0,
                    probes: Vec::new(),
                    subsumed: None,
                    // Empty for an upstream-propagated skip (`own == false`)
                    // — only the model that itself declared+resolved the
                    // per-cell skip names its own addresses.
                    deferred_cells: deferral_skipped_cells
                        .get(&plan.name)
                        .cloned()
                        .unwrap_or_default(),
                },
            );
            reporter.model_completed(run_id, &plan.name, 0, std::time::Duration::ZERO);
            return Ok(ModelOutcome::Completed(ModelSuccess {
                manifest_entries,
                check_results,
                skip_set,
                rows: 0,
            }));
        }

        // ── Skip set: skip models downstream of a failed error check ─────
        if request.run_checks && already_skip {
            tracing::info!(
                "Skipping model '{}' — downstream of a failed error-severity check",
                plan.name
            );
            manifest_entries.insert(
                plan.name.clone(),
                smelt_state::ModelRunRecord {
                    strategy: "skipped_failed_check".to_string(),
                    time_range: None,
                    partitions_updated: vec![],
                    row_count: 0,
                    duration_ms: 0,
                    batch_safety: Some("skipped".to_string()),
                    outcome: smelt_state::RunOutcomeKind::Skipped,
                    definition_hash: compute_model_hash(&plan.sql),
                    error: None,
                    retry_count: 0,
                    probes: Vec::new(),
                    subsumed: None,
                    deferred_cells: Vec::new(),
                },
            );
            reporter.model_completed(run_id, &plan.name, 0, std::time::Duration::ZERO);
            return Ok(ModelOutcome::Completed(ModelSuccess {
                manifest_entries,
                check_results,
                skip_set,
                rows: 0,
            }));
        }

        reporter.model_started(run_id, &plan.name, model_idx, models_total);

        let model_start = Instant::now();
        let mut total_rows = 0usize;

        let model_target = &target_assignments[&plan.name];
        let backend = backends[model_target].as_ref();
        let schema = &config.targets[model_target].schema;
        let availability = availability_for_target(state_availability, model_target, config);

        // The deployed-schema snapshot's column names, captured BEFORE the
        // schema-evolution gate below runs `check_and_migrate` (which, on
        // an `AlterTable` migration, updates the stored schema forward to
        // already include the new column) — this is the "old" snapshot the
        // definition-change trigger (`Trigger::ColumnAdded`) diffs the
        // model's current SQL against, read once here so the later
        // `resolve_live_in_place_update_cell` call sees the schema as it
        // was BEFORE this run's own ALTER, not after
        // (`docs/plans/20260809-sensitivity-precision.md` Phase 6). Empty
        // when no deployed schema exists yet (first run) — fail-closed, no
        // trigger derived, same as `smelt-db`'s own diagnostic path.
        let deployed_column_names: Vec<String> = file_store
            .load_schema(&plan.model_file.db_name_owned())
            .ok()
            .flatten()
            .map(|s| s.columns.into_iter().map(|c| c.name).collect())
            .unwrap_or_default();

        // The `Trigger::ColumnAdded` → `Technique::InPlaceUpdate` cell,
        // resolved ONCE here — before the schema-evolution gate runs its
        // `ALTER TABLE` — so its backfill assignments can be folded into
        // the SAME `StatementGroup` as the `ADD COLUMN` below
        // (`docs/plans/20260809-sensitivity-precision.md` Phase 6 review
        // finding: a crash between the migration's `save_schema` and a
        // standalone backfill dispatch left the column permanently NULL
        // with no repair path, since the next run's snapshot already
        // contains the column and the trigger never re-derives). Reused,
        // never re-derived, by both the migration gate below and the
        // fallback standalone dispatch after it.
        let clean_sql_for_definition_change = smelt_parser::strip_frontmatter(&plan.sql);
        let in_place_update_cell = if plan.incremental.is_some() && !deployed_column_names.is_empty()
        {
            let (in_place_sources, _) = build_maint_source_facts(&plan.model_file, source_infos);
            plan.model_file.metadata.as_deref().and_then(|metadata| {
                crate::maintenance_driver::resolve_live_in_place_update_cell(
                    &clean_sql_for_definition_change,
                    &plan.model_file.db_name_owned(),
                    metadata,
                    &in_place_sources,
                    &deployed_column_names,
                    &availability,
                )
            })
        } else {
            None
        };

        // ── Definition-delta gate (`docs/specs/definition_deltas.md` §"Detection") ──
        // A maintained (incremental) model whose stored table already exists
        // refuses to fold a data delta over a pending, non-eclipsed,
        // unapproved definition delta rather than silently maintaining a
        // table whose definition no longer matches its contents.
        // `--full-refresh` is not a fold and is never gated; `--dry-run`
        // executes nothing to gate. Detection failures degrade to a warning
        // and the run proceeds — never break a run on a diff this module
        // cannot factor.
        if plan.incremental.is_some() && !request.full_refresh && !request.dry_run {
            if let Ok(true) = backend
                .table_exists(schema, &plan.model_file.db_name_owned())
                .await
            {
                let status = {
                    let db_guard = db.lock().await;
                    crate::definition_delta::detect_definition_delta(
                        file_store,
                        &plan.model_file,
                        all_models,
                        legacy_sources_config.as_ref(),
                        &db_guard,
                    )
                };
                match status {
                    // A pure column addition is exempt: the maintenance
                    // driver's own live `Trigger::ColumnAdded` dispatch
                    // (below, the "Definition-change trigger" fallback)
                    // already handles this shape safely and atomically as
                    // part of an ordinary run — this is the documented
                    // narrower third mechanism (`docs/specs/
                    // definition_deltas.md` §"Detection") that predates and
                    // coexists with `smelt migrate`.
                    Ok(crate::definition_delta::DefinitionDeltaStatus::Pending {
                        pure_column_addition: true,
                        ..
                    }) => {}
                    Ok(crate::definition_delta::DefinitionDeltaStatus::Pending {
                        verdict,
                        plan_hash,
                        ..
                    }) => {
                        return Err(crate::definition_delta::DefinitionDeltaPendingError {
                            model: plan.name.clone(),
                            verdict,
                            plan_hash,
                        }
                        .into());
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(
                            "Definition-delta detection failed for '{}': {}. Continuing.",
                            plan.name,
                            e
                        );
                    }
                }
            }
        }

        // ── Schema evolution gate (incremental models only) ──────────────
        // For incremental models that have a deployed schema, check whether
        // the inferred columns have changed and apply (or block) the required
        // migration. `force_full_refresh` overrides the planned incremental
        // strategy to a full-table rebuild when evolution requires it.
        let mut force_full_refresh = false;
        // Columns whose `InPlaceUpdate` backfill was already folded into
        // and executed as part of the migration's own `StatementGroup`
        // below — the standalone dispatch after this gate must skip these
        // (idempotency: never re-run the same backfill twice).
        let mut migration_backfilled_columns: Vec<String> = Vec::new();
        if plan.incremental.is_some() {
            let evolution_strategy = plan
                .model_file
                .metadata
                .as_deref()
                .and_then(|m| m.schema_evolution.as_ref())
                .map(|se| &se.strategy);
            let use_alter = !matches!(
                evolution_strategy,
                Some(smelt_core::metadata::SchemaEvolutionStrategy::FullRefresh)
            );

            if use_alter {
                if let Ok(true) = backend
                    .table_exists(schema, &plan.model_file.db_name_owned())
                    .await
                {
                    let inferred_columns = {
                        let db_guard = db.lock().await;
                        infer_deployed_columns(&db_guard, &plan.model_file)
                    };
                    if !inferred_columns.is_empty() {
                        let db_table_name = plan.model_file.db_name_owned();
                        let (column_defaults, mut backfill_exprs) =
                            extract_evolution_maps(plan.model_file.metadata.as_deref());
                        // Fold the derived `InPlaceUpdate` cell's own
                        // backfill assignments into the SAME map the
                        // declared `backfill:` directive mechanism already
                        // uses — `check_and_migrate`/`plan_migration_for_
                        // backend` (`schema_tracking.rs`) emits the
                        // `ADD COLUMN` and its `UPDATE ... SET` into ONE
                        // `StatementGroup`, so routing the derived
                        // assignment through this map makes it atomic with
                        // the migration for free, reusing the existing
                        // atomic mechanism rather than re-authoring it.
                        // A user's explicit `default:`/`backfill:`
                        // directive always wins (checked first) — the
                        // derived assignment only fills a gap the user
                        // left undeclared.
                        if let Some((_cell, assignments)) = &in_place_update_cell {
                            for (col, expr) in assignments {
                                if !column_defaults.contains_key(col)
                                    && !backfill_exprs.contains_key(col)
                                {
                                    backfill_exprs.insert(col.clone(), expr.clone());
                                }
                            }
                        }
                        let target_config = config
                            .targets
                            .get(model_target)
                            .expect("target config must exist");
                        let table_format = config.get_format(
                            &plan.name,
                            plan.model_file.metadata.as_deref(),
                            target_config,
                        );
                        let ddl_backend =
                            ddl_backend_for_dialect(backend.dialect(), table_format, None);
                        let schema_evolution_retry =
                            RetryPolicy::from_request(request, run_id, &plan.name, reporter);
                        match check_and_migrate(
                            backend,
                            file_store,
                            &db_table_name,
                            &plan.sql,
                            schema,
                            &inferred_columns,
                            request.allow_column_removal,
                            request.allow_full_refresh,
                            request.dry_run,
                            &column_defaults,
                            &backfill_exprs,
                            Some(&ddl_backend),
                            &schema_evolution_retry,
                        )
                        .await
                        {
                            Ok(result) => {
                                if let SchemaEvolutionResult::Migrated {
                                    backfilled_columns,
                                    ..
                                } = &result
                                {
                                    migration_backfilled_columns = backfilled_columns.clone();
                                }
                                match crate::safety::should_force_full_refresh(
                                    &result,
                                    &plan.name,
                                    request.allow_column_removal,
                                    request.allow_full_refresh,
                                ) {
                                    Ok(should_refresh) => force_full_refresh = should_refresh,
                                    Err(e) => {
                                        return Err(e);
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Schema evolution check failed: {}. Continuing with incremental.",
                                    e
                                );
                            }
                        }
                    }
                }
            } else if let Ok(true) = backend
                .table_exists(schema, &plan.model_file.db_name_owned())
                .await
            {
                // `schema_evolution: strategy: full_refresh` opts out of
                // `ALTER`-based evolution — it has no migration path of its
                // own, so a schema change forces a rebuild under the new
                // definition instead of silently taking neither route
                // (`docs/specs/definition_deltas.md` §"The atomicity rule").
                let inferred_columns = {
                    let db_guard = db.lock().await;
                    infer_deployed_columns(&db_guard, &plan.model_file)
                };
                if !inferred_columns.is_empty() {
                    if let Ok(Some(deployed_schema)) =
                        file_store.load_schema(&plan.model_file.db_name_owned())
                    {
                        if full_refresh_escape_requires_rebuild(
                            evolution_strategy,
                            &deployed_schema.columns,
                            &inferred_columns,
                        ) {
                            tracing::warn!(
                                "Schema changed for '{}' under schema_evolution: strategy: \
                                 full_refresh — rebuilding under the new definition instead of \
                                 an ALTER migration.",
                                plan.name
                            );
                            force_full_refresh = true;
                        }
                    }
                }
            }
        }

        // ── Definition-change trigger (Trigger::ColumnAdded → Technique::
        // InPlaceUpdate) ───────────────────────────────────────────────────
        // Runs once per incremental model per run, common to both the keyed
        // and non-keyed branches below — a one-time migration-style
        // backfill over the model's existing rows, orthogonal to whichever
        // window/creation/mutation technique the rest of this run
        // dispatches (`docs/specs/definition_deltas.md` §"The verdict per
        // column group"). The cell was already resolved once, above, before
        // the migration gate ran — reused here, never re-derived. Any
        // column the migration gate already folded into its own
        // `StatementGroup` (`migration_backfilled_columns`) is skipped —
        // re-dispatching it here would be a redundant, non-atomic re-run of
        // a backfill that already committed atomically with its `ADD
        // COLUMN`. A column that was NOT folded (the migration gate didn't
        // run this run — e.g. `schema_evolution: strategy: full_refresh`,
        // or a column the gate couldn't fold) is never backfilled by a
        // standalone `UPDATE` against a schema that may not carry it: the
        // model force-full-refreshes instead, which is always correct and
        // always atomic (`docs/specs/definition_deltas.md` §"The atomicity
        // rule").
        let mut used_in_place_update = false;
        if let Some((_cell, assignments)) = &in_place_update_cell {
            let remaining: Vec<(String, String)> = assignments
                .iter()
                .filter(|(col, _)| !migration_backfilled_columns.iter().any(|c| c == col))
                .cloned()
                .collect();
            if remaining.len() != assignments.len() {
                used_in_place_update = true; // some/all columns already backfilled atomically above
            }
            if !remaining.is_empty() {
                if let Ok(true) = backend
                    .table_exists(schema, &plan.model_file.db_name_owned())
                    .await
                {
                    tracing::warn!(
                        "Definition-change backfill for '{}' (columns: {:?}) could not be \
                         folded into an atomic migration this run — forcing a full refresh \
                         instead of a non-atomic standalone UPDATE.",
                        plan.name,
                        remaining.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>()
                    );
                    force_full_refresh = true;
                }
                // Table absent: no rows to backfill, nothing to dispatch.
            }
        }

        // Keyed dispatch — handled separately from the incremental /
        // full-refresh branches because it has its own per-partition merge
        // loop (see `smelt_runtime::cumulative` and
        // `docs/specs/incremental_models.md`).
        let plan_is_keyed = plan
            .model_file
            .metadata
            .as_deref()
            .is_some_and(|m| m.is_keyed());
        if plan_is_keyed {
            let db_table_name = plan.model_file.db_name_owned();
            let compiler = compilers.get(model_target);
            let resolver = &ephemeral_resolvers[model_target];

            // W10 Phase 4 (`docs/plans/20260720-prod-w10-keyed-mutable-
            // admission.md`): consult the SAME derived `MaintenancePlan`
            // the non-keyed incremental branch already does (below, ~L1788)
            // for a live `ColumnScopedMerge` cell on one of this model's
            // `explicitly_mutable` sources — the keyed run loop's own
            // analogue of `resolve_incremental_strategy`. The cumulative
            // fold above/below still owns the `NewData` (creation/append)
            // trigger; this owns the `UpstreamMutation` trigger, dispatched
            // ALONGSIDE it once a live cell resolves and the target table
            // already exists (never on the creation run — there is nothing
            // to merge into yet). `table_exists_before_run` is captured
            // BEFORE the fold below runs (which may itself create the
            // table on a first run), mirroring the non-keyed branch's own
            // "did the table exist before THIS run" capture.
            let clean_sql_for_merge = smelt_parser::strip_frontmatter(&plan.sql).to_string();
            let (maint_source_facts, explicitly_mutable) =
                build_maint_source_facts(&plan.model_file, source_infos);
            let table_exists_before_run = backend
                .table_exists(schema, &db_table_name)
                .await
                .unwrap_or(false);
            let column_scoped_cell = match plan.model_file.metadata.as_deref() {
                Some(metadata) => crate::maintenance_driver::resolve_live_column_scoped_cell(
                    &clean_sql_for_merge,
                    &db_table_name,
                    metadata,
                    &maint_source_facts,
                    &explicitly_mutable,
                    backend.capabilities().supports_column_scoped_merge,
                    &request.technique_overrides,
                    &availability,
                )?,
                None => None,
            };
            // Membership-sensitive counterpart of `column_scoped_cell` above
            // (`docs/plans/20260808-membership-sensitivity.md` Phase 2): a
            // live `Technique::DeleteInsert` cell over a proven keyed row
            // identity, dispatched through the staged-candidate conditional
            // recompute instead of a column-scoped `MERGE`. Mutually
            // exclusive with `column_scoped_cell` by construction — the two
            // resolvers filter on disjoint `Technique`s of the SAME derived
            // plan, so at most one of them is ever `Some` for a given
            // `explicitly_mutable` source set.
            let membership_recompute_cell = match plan.model_file.metadata.as_deref() {
                Some(metadata) => {
                    crate::maintenance_driver::resolve_live_membership_recompute_cell(
                        &clean_sql_for_merge,
                        &db_table_name,
                        metadata,
                        &maint_source_facts,
                        &explicitly_mutable,
                        &request.technique_overrides,
                        &availability,
                    )?
                }
                None => None,
            };
            // The repair family's counterpart of the two resolvers above
            // (`docs/specs/incremental_models.md` §"The repair family").
            // Unlike them it serves the model's OWN driving/fold trigger:
            // `derive_new_data`'s key-grain branch narrows a faithful-fold
            // source-posture refusal into a `Technique::PerGroupRecompute`
            // cell on that same `Trigger::NewData { source }`, so the repair
            // cell is an ALTERNATIVE to the `KeyedFold` cell rather than a
            // technique dispatched alongside the fold. That is why it is
            // routed inside the window-forward branch below — *instead of*
            // `execute_cumulative_aggregate` — rather than at the
            // post-fold dispatch sites the column-scoped-merge and
            // membership-recompute cells use: folding a retracted
            // contribution first and repairing after would already have
            // corrupted stored state. It is still ordered after those two
            // in the ladder in the sense that matters — a source they
            // already claim carries an `UpstreamMutation` cell, never a
            // repair cell: `derive_mutation` narrows a source already
            // covered by a `Trigger::NewData`/`Technique::PerGroupRecompute`
            // repair cell out of the wider `UpstreamMutation` admission rule
            // (`derive_triggers`'s own doc comment — the clock is NOT part
            // of that rule since phase 19), so the two can never both be
            // derived for the same source in the first place.
            let per_group_recompute_cell = match plan.model_file.metadata.as_deref() {
                Some(metadata) => {
                    crate::maintenance_driver::resolve_live_per_group_recompute_cell(
                        &clean_sql_for_merge,
                        &db_table_name,
                        metadata,
                        &maint_source_facts,
                        &explicitly_mutable,
                        &request.technique_overrides,
                        backend.dialect(),
                        backend.capabilities().supports_fingerprint_sidecar,
                        &availability,
                    )?
                }
                None => None,
            };
            // A key-addressed model-edge cell (`docs/specs/incremental_models.md`
            // §"Upstream model edges"): an upstream maintained model whose own
            // derived output-delta shape is `KeyedUpsert` folds via the repair
            // family's own `Technique::PerGroupRecompute`, restricted to the
            // upstream's affected key set rather than a source's `ScanClamp` —
            // the sibling of `per_group_recompute_cell` above for this
            // model's upstream MODEL edges rather than its declared sources.
            let keyed_model_edges = model_edges_for(&plan.model_file, model_by_addr, source_infos);
            let key_edge_dispatch = resolve_and_dispatch_key_addressed_edge_cell(
                backend,
                schema,
                &plan.name,
                &plan.model_file,
                &clean_sql_for_merge,
                &db_table_name,
                &maint_source_facts,
                &explicitly_mutable,
                &keyed_model_edges,
                table_exists_before_run,
                model_by_addr,
                config,
                request,
                compiler,
                resolver,
                run_id,
                reporter,
                &availability,
            )
            .await?;

            // Classify up front, regardless of window presence, so the
            // derived run shape (`docs/specs/incremental_shapes.md` §"The
            // two run shapes") can gate which branch below is even
            // reachable — a classifier rejection must REFUSE the model
            // (§"Key-grain constraints" #4 — "The catalogue is closed and
            // the classifier is fail-closed"), never silently fall back to
            // a full refresh.
            let clean_sql_for_classify = smelt_parser::strip_frontmatter(&plan.sql);
            let model_has_timeseries = plan
                .model_file
                .metadata
                .as_ref()
                .is_some_and(|m| m.timeseries.is_some());
            let declared_fds: &[smelt_core::config::FunctionalDependency] = plan
                .model_file
                .metadata
                .as_ref()
                .map(|m| m.functional_dependencies.as_slice())
                .unwrap_or(&[]);
            let classification = crate::cumulative::classify_cumulative_sql(
                &plan.name,
                &clean_sql_for_classify,
                source_timeseries,
                model_has_timeseries,
                declared_fds,
            )?;

            let mut used_per_group_recompute = false;
            let mut used_diff_patch = false;
            // Populated only by the snapshot-reconcile arm's declared
            // `contract.retain_departed` point (phase 34): the reconcile
            // anti-join probe's outcome, threaded to this model's manifest
            // entry below instead of being dropped after only a
            // `tracing::info!`.
            let mut retain_departed_probes: Vec<smelt_state::ProbeRecord> = Vec::new();
            // A key-addressed model-edge cell has no run-window axis at all
            // (its bounded read is the upstream's own affected key set, not
            // an interval) — checked BEFORE the `(start_date, end_date)`
            // dispatch below rather than nested inside its window-forward
            // arm, so it fires regardless of which run shape this model's
            // OWN driving trigger classifies as (a clockless upstream
            // typically drives the downstream into the snapshot-reconcile
            // shape, which has no run window to match on at all). It is
            // derived for a DIFFERENT trigger source (the upstream model's
            // own bare name) than any declared-source repair cell, so the
            // two can never contend for the same trigger. Never on the
            // creation run — there is nothing to repair yet, and the fold's
            // own create path is what materializes the table
            // (`table_exists_before_run` was captured before any of this
            // model's writes).
            let exec_result = match key_edge_dispatch {
                Some(dispatch) => {
                    used_per_group_recompute = dispatch.used_per_group_recompute;
                    used_diff_patch = dispatch.used_diff_patch;
                    Ok(dispatch.result)
                }
                None => match (start_date, end_date) {
                (Some(s), Some(e)) => {
                    if classification.is_snapshot_reconcile() {
                        anyhow::bail!(
                            "Model '{}' derives the snapshot-reconcile run shape (no clocked \
                             driving source, `docs/specs/incremental_models.md` §\"The two run \
                             shapes\") — --event-time-start/--event-time-end are not accepted; \
                             run without an event-time window instead.",
                            plan.name
                        );
                    }
                    let time_range = TimeRange {
                        start: s.format("%Y-%m-%d").to_string(),
                        end: e.format("%Y-%m-%d").to_string(),
                        axis: smelt_logical::PartitionAxis::Calendar,
                    };
                    let retry_policy =
                        RetryPolicy::from_request(request, run_id, &plan.name, reporter);
                    // The repair family displaces the fold for this
                    // trigger, never runs after it: an admitted
                    // `Technique::PerGroupRecompute` cell exists precisely
                    // because the faithful-fold source-posture obligation
                    // FAILED for this source, so folding the delta in would
                    // be the unsound write the repair exists to avoid. Never
                    // on the creation run — there is nothing to repair yet,
                    // and the fold's own create path is what materializes
                    // the table (`table_exists_before_run` was captured
                    // before any of this model's writes).
                    match per_group_recompute_cell
                        .as_ref()
                        .filter(|_| table_exists_before_run)
                    {
                        Some((source, _cell, key, slice, write, discovery)) => {
                            // A cell that resolved live but whose emitter
                            // inputs cannot be built errors by name — never
                            // a silent fall-through to the fold.
                            let source_info = source_infos
                                .iter()
                                .find(|info| {
                                    let segs = &info.address_segments;
                                    let bare = match segs.split_first() {
                                        Some((first, rest)) if first == "sources" => rest.join("."),
                                        _ => segs.join("."),
                                    };
                                    &bare == source
                                })
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "keyed run path: model '{}' resolved a live per-group \
                                         recompute cell on source '{source}', but that source \
                                         has no resolved physical table — the affected-key read \
                                         cannot be built",
                                        plan.name
                                    )
                                })?;
                            let source_table =
                                source_info.db_name_for_target(model_target, schema);
                            // Declared here (not inside the `discovery`
                            // match arm below) purely so it outlives the
                            // `RepairSidecarRefresh` borrow the SidecarDiff
                            // leg constructs — the format is only ever
                            // consumed on that leg.
                            let source_address = format!("smelt.sources.{source}");
                            let consumer_address =
                                format!("smelt.models.{}", plan.model_file.canonical_path());
                            // P9 (`docs/specs/incremental_models.md` §"The
                            // repair family" — "Obligation 7 over a
                            // `mutable_snapshot` source"): a
                            // `RepairDiscovery::SidecarDiff` cell reads the
                            // group-grain sidecar diff instead of the
                            // clamped current-source scan — unbounded by
                            // `slice`, per that section's own rationale —
                            // and its result is turned into the SAME
                            // one-column `delta_key` relation shape every
                            // downstream repair builder expects.
                            let (affected_keys_select, sidecar_refresh) = match discovery {
                                crate::maintenance_driver::RepairDiscovery::ClampedScan => {
                                    // Typed literals, unlike the bare quoted
                                    // strings every other `Region`
                                    // construction in this file uses: the
                                    // repair's affected-key read is the one
                                    // place a region endpoint is an
                                    // *operand* (`widened_scan_predicate`
                                    // subtracts the clamp's margin from
                                    // it), and a bare string literal minus
                                    // an INTERVAL is ambiguous to the
                                    // binder rather than implicitly a
                                    // timestamp.
                                    let region = smelt_logical::maintenance::emit::Region {
                                        start: format!("TIMESTAMP '{}'", s.format("%Y-%m-%d")),
                                        end: format!("TIMESTAMP '{}'", e.format("%Y-%m-%d")),
                                    };
                                    let select =
                                        crate::maintenance_driver::repair_affected_keys_select(
                                            &source_table,
                                            key,
                                            Some(slice),
                                            &region,
                                        );
                                    (select, None)
                                }
                                crate::maintenance_driver::RepairDiscovery::SidecarDiff {
                                    digest_columns,
                                } => {
                                    let output_table = format!("{schema}.{db_table_name}");
                                    let keys =
                                        crate::maintenance_driver::diff_repair_group_sidecar_changed_keys(
                                            backend,
                                            schema,
                                            &source_address,
                                            &source_table,
                                            &output_table,
                                            key,
                                            digest_columns,
                                            &clean_sql_for_merge,
                                            &consumer_address,
                                        )
                                        .await?;
                                    let select =
                                        crate::maintenance_driver::repair_keys_literal_select(
                                            &keys,
                                            smelt_backend::maintenance_dialect(backend.dialect()),
                                        );
                                    let refresh = crate::maintenance_driver::RepairSidecarRefresh {
                                        schema,
                                        source_address: &source_address,
                                        source_table: &source_table,
                                        group_key: key,
                                        digest_columns,
                                        model_sql: &clean_sql_for_merge,
                                        consumer_address: &consumer_address,
                                    };
                                    (select, Some(refresh))
                                }
                            };
                            // The model's own FULL, unwindowed recompute —
                            // the same `clean_sql_for_merge` the
                            // membership-recompute dispatch below compiles,
                            // for the same reason: a repaired group must
                            // equal a full refresh of that group.
                            //
                            // Widened with `classification`'s own hidden
                            // decomposed-state columns (P10,
                            // `docs/outcomes/20260809-repair-family/phases/
                            // 10-plan.md`) BEFORE compiling — a decomposed
                            // combiner's create/merge path already carries
                            // those `__`-marked columns in the physical
                            // table, so the repair's candidate/insert must
                            // supply them too, or the `INSERT`'s implicit
                            // column list mismatches the table. Raw,
                            // pre-compile SQL, same ordering rationale as
                            // `execute_snapshot_reconcile`. A no-op for
                            // every stateless column family.
                            let state_columns = classification.state_columns();
                            let augmented_sql = crate::maintenance_driver::repair_augmented_model_sql(
                                &clean_sql_for_merge,
                                &state_columns,
                            )?;
                            let compiled = compiler.compile_with_sql_and_ephemerals(
                                &plan.model_file,
                                schema,
                                &augmented_sql,
                                resolver,
                            )?;
                            let candidate_select =
                                crate::maintenance_driver::repair_candidate_select(
                                    &compiled.sql,
                                    key,
                                    &affected_keys_select,
                                );
                            match write {
                                crate::maintenance_driver::RepairWrite::TargetedDeleteInsert => {
                                    used_per_group_recompute = true;
                                    crate::maintenance_driver::execute_per_group_recompute(
                                        backend,
                                        schema,
                                        &db_table_name,
                                        key,
                                        &affected_keys_select,
                                        &candidate_select,
                                        &retry_policy,
                                        sidecar_refresh.as_ref(),
                                    )
                                    .await
                                }
                                crate::maintenance_driver::RepairWrite::DiffPatch {
                                    compared_columns,
                                    delete_leg,
                                } => {
                                    let slice_predicate =
                                        crate::maintenance_driver::repair_slice_predicate(
                                            &db_table_name,
                                            key,
                                            &affected_keys_select,
                                        );
                                    // A group whose PRESENTED value is
                                    // unchanged but whose hidden state moved
                                    // must still be rewritten — comparing
                                    // only the presented columns would
                                    // suppress the write and leave stale
                                    // state behind a correct-looking value
                                    // (strictly less suppression than
                                    // presented-only, sound by
                                    // construction).
                                    let mut compared_columns = compared_columns.clone();
                                    compared_columns
                                        .extend(state_columns.iter().map(|sc| sc.name.clone()));
                                    used_diff_patch = true;
                                    crate::maintenance_driver::execute_diff_patch(
                                        backend,
                                        schema,
                                        &db_table_name,
                                        key,
                                        &candidate_select,
                                        &compared_columns,
                                        &slice_predicate,
                                        delete_leg,
                                        &retry_policy,
                                        sidecar_refresh.as_ref(),
                                    )
                                    .await
                                }
                            }
                        }
                        None => {
                            crate::cumulative::execute_cumulative_aggregate(
                                backend,
                                &plan.model_file,
                                compilers,
                                resolver,
                                model_target,
                                schema,
                                &db_table_name,
                                &time_range,
                                source_timeseries,
                                source_key_recurrence,
                                false,
                                &retry_policy,
                                &probe_policy_for_model(config, prior_runs, &plan.name),
                            )
                            .await
                        }
                    }
                }
                _ if classification.is_snapshot_reconcile() => {
                    // No run window, snapshot-reconcile run shape: whole-
                    // source keyed MERGE (create-if-missing, retained-
                    // departed-keys reconcile otherwise) — never the
                    // unconditional drop+create the window-forward branch
                    // below uses, which would silently drop departed keys.
                    crate::cumulative::execute_snapshot_reconcile(
                        backend,
                        &plan.model_file,
                        compilers,
                        resolver,
                        model_target,
                        schema,
                        &db_table_name,
                        &classification,
                        &mut retain_departed_probes,
                    )
                    .await
                }
                _ if !request.full_refresh => {
                    // No run window, window-forward run shape, no
                    // `--full-refresh`: refuse rather than silently
                    // drop+recreating from a whole-source SELECT
                    // (`docs/specs/incremental_shapes.md` §"The key grain" —
                    // a window-forward keyed run requires both
                    // `--event-time-start`/`--event-time-end`; the
                    // `--full-refresh` escape below is the only intentional
                    // rebuild path). Mirrors the snapshot-reconcile arm
                    // above's plain-`bail!` shape.
                    anyhow::bail!(
                        "Model '{}' derives the window-forward run shape (a clocked driving \
                         source, `docs/specs/incremental_models.md` §\"The two run shapes\") \
                         and was run with no event-time window — both \
                         --event-time-start/--event-time-end are required, or pass \
                         --full-refresh to intentionally rebuild the whole target from a \
                         whole-source SELECT (`docs/specs/incremental_shapes.md` §\"The key \
                         grain\").",
                        plan.name
                    );
                }
                _ => {
                    // No run window, window-forward run shape,
                    // `--full-refresh`: single-shot full refresh of the
                    // keyed SELECT. Matches CLI's behaviour for
                    // `smelt build` / `smelt run --full-refresh` without an
                    // event-time window.
                    let clean_sql = smelt_parser::strip_frontmatter(&plan.sql);
                    let compiled = compiler.compile_with_sql_and_ephemerals(
                        &plan.model_file,
                        schema,
                        &clean_sql,
                        resolver,
                    )?;
                    backend
                        .drop_table_if_exists(schema, &db_table_name)
                        .await
                        .map_err(|err| {
                            anyhow::anyhow!("Failed to drop {}: {}", db_table_name, err)
                        })?;
                    backend
                        .create_table_as(schema, &db_table_name, &compiled.sql)
                        .await
                        .map_err(|err| {
                            anyhow::anyhow!("Failed to create keyed model {}: {}", plan.name, err)
                        })?;
                    let row_count = backend
                        .get_row_count(schema, &db_table_name)
                        .await
                        .unwrap_or(0);
                    Ok(smelt_backend::ExecutionResult {
                        model_name: plan.name.clone(),
                        duration: StdDuration::from_millis(0),
                        row_count,
                        preview: None,
                    })
                }
                }
                }
            ;

            let exec_result = match exec_result {
                Ok(r) => r,
                Err(e) => {
                    return Err(e);
                }
            };

            // P9 task 6 (`docs/outcomes/20260809-repair-family/phases/
            // 09-plan.md`): seed the group-grain fingerprint sidecar's
            // initial comparandum on THIS run's own creation
            // (`!table_exists_before_run` — the fold above just
            // materialized the table). Without this, the first live repair
            // after creation would find no partition at all and take the
            // absent-comparandum degradation (task 7) on every run rather
            // than just the very first one. `write_group` is an empty,
            // non-transactional group — there is no consuming write to ride
            // alongside here, only a baseline populate.
            if !table_exists_before_run {
                if let Some((
                    source,
                    _cell,
                    group_key,
                    _slice,
                    _write,
                    crate::maintenance_driver::RepairDiscovery::SidecarDiff { digest_columns },
                )) = per_group_recompute_cell.as_ref()
                {
                    if let Some(source_info) = source_infos.iter().find(|info| {
                        let segs = &info.address_segments;
                        let bare = match segs.split_first() {
                            Some((first, rest)) if first == "sources" => rest.join("."),
                            _ => segs.join("."),
                        };
                        &bare == source
                    }) {
                        let source_table = source_info.db_name_for_target(model_target, schema);
                        let source_address = format!("smelt.sources.{source}");
                        let consumer_address =
                            format!("smelt.models.{}", plan.model_file.canonical_path());
                        let empty_group = smelt_logical::maintenance::emit::StatementGroup {
                            statements: vec![],
                            transactional: false,
                        };
                        crate::maintenance_driver::refresh_repair_group_sidecar(
                            backend,
                            schema,
                            &source_address,
                            &source_table,
                            group_key,
                            digest_columns,
                            &clean_sql_for_merge,
                            &consumer_address,
                            &empty_group,
                        )
                        .await?;
                    }
                }
            }

            // W10 Phase 4: dispatch the live `UpstreamMutation` cell
            // resolved above, alongside the cumulative fold that just ran —
            // "the driver loop becomes the per-cell technique executor"
            // (`docs/plans/20260707-maintenance-plan-impl.md` MP11), now
            // extended to the keyed run loop. Never fires on the creation
            // run (`table_exists_before_run` was captured before the fold
            // above could have just created the table).
            let mut used_column_scoped_merge = false;
            if let Some((source, cell, suppression)) = column_scoped_cell.as_ref() {
                if table_exists_before_run {
                    // Mutation-happened discrimination
                    // (`docs/specs/incremental_models.md` §"When a mutation
                    // cell dispatches"): before dispatching the technique
                    // this live `UpstreamMutation` cell licenses, compare
                    // `source`'s current whole-source fingerprint against
                    // its recorded baseline. `None` here (no declared
                    // columns for the source) is treated the same as
                    // `Dispatch` — nothing to fingerprint means no evidence
                    // to skip on, same fail-open-to-dispatch posture the
                    // spec's "no recorded baseline at all" clause takes.
                    let mutation_gate =
                        resolve_upstream_mutation_gate(
                            backend,
                            &plan.name,
                            source_infos,
                            source,
                            model_target,
                            schema,
                            file_store,
                            state_io_lock,
                        )
                        .await?;
                    let mutation_should_dispatch = !matches!(
                        mutation_gate.as_ref().map(|(v, _)| v),
                        Some(crate::mutation_probe::MutationVerdict::NoOp)
                    );
                    if mutation_should_dispatch {
                    // The mutated dimension's own declared `unique_key`
                    // (`sources.md` §"Row identity") — same lookup the
                    // non-keyed incremental branch performs, needed only for
                    // the horizon-clamped corner's join-contribution proof.
                    let dimension_unique_key: Vec<String> = source_infos
                        .iter()
                        .find(|info| {
                            let segs = &info.address_segments;
                            let bare = match segs.split_first() {
                                Some((first, rest)) if first == "sources" => rest.join("."),
                                _ => segs.join("."),
                            };
                            &bare == source
                        })
                        .and_then(|info| info.unique_key.clone())
                        .unwrap_or_default();
                    let contribution = if matches!(
                        cell.partition_local,
                        smelt_logical::maintenance::PartitionLocal::Yes
                    ) {
                        crate::maintenance_driver::dimension_join_contribution(
                            &clean_sql_for_merge,
                            source,
                            &dimension_unique_key,
                        )
                    } else {
                        smelt_logical::analysis::join_shape::ContributionVerdict::Monotone
                    };
                    let model_unique_key: Vec<String> = plan
                        .model_file
                        .metadata
                        .as_deref()
                        .and_then(|m| m.unique_key.clone())
                        .unwrap_or_default();
                    let dispatch = crate::maintenance_driver::decide_column_merge_dispatch(
                        cell,
                        source,
                        table_exists_before_run,
                        !model_unique_key.is_empty(),
                        &contribution,
                    );
                    if let Some(dispatch) = dispatch {
                        let compiled = compiler.compile_with_sql_and_ephemerals(
                            &plan.model_file,
                            schema,
                            &clean_sql_for_merge,
                            resolver,
                        )?;
                        let (window_start, window_end) = match (start_date, end_date) {
                            (Some(s), Some(e)) => (
                                s.format("%Y-%m-%d").to_string(),
                                e.format("%Y-%m-%d").to_string(),
                            ),
                            _ => (String::new(), String::new()),
                        };
                        // A bare `grain: key` output has no partition
                        // column of its own — `column: String::new()` is
                        // the documented empty-string convention
                        // `execute_column_scoped_write_with_observed_delta`
                        // already reads as "no partition column" (T5's
                        // observed-delta recording still keys on
                        // `[start, end)` alone).
                        let window = smelt_backend::PartitionRange {
                            column: String::new(),
                            start: window_start,
                            end: window_end,
                            axis: smelt_backend::PartitionAxis::Calendar,
                        };
                        let retry_policy =
                            RetryPolicy::from_request(request, run_id, &plan.name, reporter);
                        let merge_result = match dispatch {
                            crate::maintenance_driver::ColumnMergeDispatch::Full => {
                                crate::maintenance_driver::execute_column_scoped_merge_full(
                                    backend,
                                    schema,
                                    &db_table_name,
                                    &model_unique_key,
                                    &compiled.sql,
                                    &compiled.output_columns,
                                    suppression,
                                    &window,
                                    &retry_policy,
                                )
                                .await
                                .map_err(|e| anyhow::anyhow!("{}", e))?
                            }
                            crate::maintenance_driver::ColumnMergeDispatch::Clamped(_) => {
                                // The horizon-clamped corner needs a
                                // conv_ts on the output's own partition
                                // axis — only a composed clock-and-identity
                                // output that ALSO declares its own
                                // `timeseries:` establishes one
                                // (`incremental_shapes.md` §"Key temporal
                                // locality"). No derivable keyed cell
                                // reaches `PartitionLocal::Yes` today (the
                                // non-keyed branch's own comment on this
                                // corner: a clocked dimension's scan-bound
                                // derivation is deferred) — refuse loudly
                                // rather than silently mis-scoping the
                                // write.
                                return Err(anyhow::anyhow!(
                                    "keyed run path: the horizon-clamped column-scoped MERGE \
                                     corner is not yet reachable for a grain: key output \
                                     ('{}') — its scan-bound derivation is deferred",
                                    plan.name
                                ));
                            }
                        };
                        used_column_scoped_merge = true;
                        total_rows = merge_result.row_count;
                        record_upstream_mutation_baseline(
                            mutation_gate,
                            source,
                            file_store,
                            state_io_lock,
                        )
                        .await;
                    }
                    }
                }
            }
            // The membership-sensitive counterpart of the block above
            // (`docs/plans/20260808-membership-sensitivity.md` Phase 2):
            // a live `Technique::DeleteInsert` cell dispatches the
            // staged-candidate conditional recompute over the model's own
            // FULL (unwindowed) recompiled SQL — the entire current
            // admitted+enriched state, not this run's time-windowed slice —
            // so a key whose row admission changed (new or changed, per
            // `resolve_live_membership_recompute_cell`'s own doc comment on
            // the departed-row limitation it inherits from the emitter) is
            // repaired. `column_scoped_cell` and `membership_recompute_cell`
            // are mutually exclusive (disjoint `Technique` filters over the
            // SAME derived plan), so this never double-dispatches a source
            // `column_scoped_cell` already handled.
            let mut used_membership_recompute = false;
            if let Some((source, cell, _group_columns, write)) =
                membership_recompute_cell.as_ref()
            {
                if table_exists_before_run && !used_column_scoped_merge {
                    let mutation_gate = resolve_upstream_mutation_gate(
                        backend,
                        &plan.name,
                        source_infos,
                        source,
                        model_target,
                        schema,
                        file_store,
                        state_io_lock,
                    )
                    .await?;
                    let mutation_should_dispatch = !matches!(
                        mutation_gate.as_ref().map(|(v, _)| v),
                        Some(crate::mutation_probe::MutationVerdict::NoOp)
                    );
                    if mutation_should_dispatch {
                    // `resolve_live_membership_recompute_cell` returns a
                    // `RowIdentity::Key` cell for the keyed staged-recompute/
                    // diff_patch legs below, or a `RowIdentity::WholeRow`
                    // cell for the keyless leg (`docs/outcomes/
                    // 20260815-definition-delta-migrate/phases/27c-plan.md`)
                    // — never a bare `Key(vec![])`, the resolver's own
                    // degenerate-proof skip.
                    let key: Option<&Vec<String>> = match &cell.row_identity.identity {
                        smelt_logical::maintenance::RowIdentity::Key(key) => Some(key),
                        smelt_logical::maintenance::RowIdentity::WholeRow => None,
                    };
                    // The model's own FULL, unwindowed recompute — same
                    // `clean_sql_for_merge` source text `column_scoped_cell`'s
                    // dispatch above compiles for its own `compiled.sql`,
                    // recompiled here independently since the two branches
                    // are mutually exclusive per-run (never both compiled).
                    let compiled = compiler.compile_with_sql_and_ephemerals(
                        &plan.model_file,
                        schema,
                        &clean_sql_for_merge,
                        resolver,
                    )?;
                    let retry_policy =
                        RetryPolicy::from_request(request, run_id, &plan.name, reporter);
                    // Same `start_date`/`end_date` → `PartitionRange`
                    // construction the column-scoped call site above uses —
                    // a bare `grain: key` output has no partition column of
                    // its own, so `column: String::new()` (T5's
                    // observed-delta recording keys on `[start, end)`
                    // alone).
                    let (window_start, window_end) = match (start_date, end_date) {
                        (Some(s), Some(e)) => (
                            s.format("%Y-%m-%d").to_string(),
                            e.format("%Y-%m-%d").to_string(),
                        ),
                        _ => (String::new(), String::new()),
                    };
                    let window = smelt_backend::PartitionRange {
                        column: String::new(),
                        start: window_start,
                        end: window_end,
                        axis: smelt_backend::PartitionAxis::Calendar,
                    };
                    let row_count = match write {
                        crate::maintenance_driver::MembershipRecomputeWrite::StagedRecompute {
                            compared_columns,
                        } => {
                            let key = key.expect(
                                "resolve_live_membership_recompute_cell only ever returns \
                                 StagedRecompute for a proven RowIdentity::Key cell",
                            );
                            crate::maintenance_driver::execute_staged_membership_recompute(
                                backend,
                                schema,
                                &db_table_name,
                                key,
                                &compiled.sql,
                                compared_columns,
                                &window,
                                &retry_policy,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("{}", e))?
                            .row_count
                        }
                        crate::maintenance_driver::MembershipRecomputeWrite::DiffPatch {
                            compared_columns,
                        } => {
                            let key = key.expect(
                                "resolve_live_membership_recompute_cell only ever returns \
                                 DiffPatch for a proven RowIdentity::Key cell",
                            );
                            // The candidate select IS the model's full
                            // admitted state — nothing is excluded from the
                            // comparison, so the slice predicate that scopes
                            // the delete leg is the trivial "whole table"
                            // predicate (`docs/outcomes/
                            // 20260815-definition-delta-migrate/
                            // phases/12-plan.md`).
                            used_diff_patch = true;
                            crate::maintenance_driver::execute_diff_patch(
                                backend,
                                schema,
                                &db_table_name,
                                key,
                                &compiled.sql,
                                compared_columns,
                                "TRUE",
                                &smelt_logical::maintenance::diff_patch::DeleteLeg::Complete,
                                &retry_policy,
                                None,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("{}", e))?
                            .row_count
                        }
                        crate::maintenance_driver::MembershipRecomputeWrite::StagedKeyless {
                            compared_columns: _,
                        } => {
                            // No key at all — this is the `RowIdentity::
                            // WholeRow` region-grained realisation
                            // (`docs/outcomes/20260815-definition-delta-migrate/
                            // phases/27c-plan.md`); `compared_columns` is the
                            // model's full payload column set, already
                            // implicit in `candidate_select`'s own shape, so
                            // the executor needs nothing further from it.
                            crate::maintenance_driver::execute_staged_keyless_recompute(
                                backend,
                                schema,
                                &db_table_name,
                                &compiled.sql,
                                &retry_policy,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("{}", e))?
                            .row_count
                        }
                    };
                    used_membership_recompute = true;
                    total_rows = row_count;
                    record_upstream_mutation_baseline(
                        mutation_gate,
                        source,
                        file_store,
                        state_io_lock,
                    )
                    .await;
                    }
                }
            }
            if !used_column_scoped_merge && !used_membership_recompute {
                total_rows = exec_result.row_count;
            }
            total_rows_overall += total_rows;
            let keyed_strategy_label = if used_in_place_update {
                "in_place_update".to_string()
            } else if used_diff_patch {
                "diff_patch".to_string()
            } else if used_per_group_recompute {
                "per_group_recompute".to_string()
            } else if used_column_scoped_merge {
                "column_scoped_merge".to_string()
            } else if used_membership_recompute {
                "delete_insert_suppressed".to_string()
            } else {
                "cumulative_aggregate".to_string()
            };
            manifest_entries.insert(
                plan.name.clone(),
                ModelRunRecord {
                    strategy: keyed_strategy_label,
                    time_range: match (start_date, end_date) {
                        (Some(s), Some(e)) => Some(TimeRangeRecord {
                            start: s.format("%Y-%m-%d").to_string(),
                            end: e.format("%Y-%m-%d").to_string(),
                        }),
                        _ => None,
                    },
                    partitions_updated: vec![],
                    row_count: total_rows,
                    duration_ms: model_start.elapsed().as_millis() as u64,
                    batch_safety: Some("cumulative".to_string()),
                    outcome: smelt_state::RunOutcomeKind::Success,
                    definition_hash: compute_model_hash(&plan.sql),
                    error: None,
                    retry_count: sink.retry_count(),
                    // Every other cumulative-arm dispatch declares no
                    // fact probe today; `retain_departed_probes` carries
                    // the snapshot-reconcile arm's declared
                    // `contract.retain_departed` probe record when that
                    // point fired (`docs/outcomes/
                    // 20260815-definition-delta-migrate/phases/
                    // 34-plan.md`), and stays empty otherwise.
                    probes: retain_departed_probes,
                    subsumed: None,
                    deferred_cells: Vec::new(),
                },
            );
            reporter.model_completed(run_id, &plan.name, total_rows, model_start.elapsed());
            // ── Check seam A: cumulative arm ─────────────────────────────────
            if request.run_checks {
                let (outcomes, to_skip) = run_model_checks(
                    &plan.name,
                    checks_by_model,
                    compilers,
                    backends,
                    target_assignments,
                    ephemeral_resolvers,
                    config.as_ref(),
                    upstream_map,
                    selected,
                    reporter,
                    run_id,
                )
                .await;
                check_results.extend(outcomes);
                skip_set.extend(to_skip);
            }
            return Ok(ModelOutcome::Completed(ModelSuccess {
                manifest_entries,
                check_results,
                skip_set,
                rows: total_rows,
            }));
        }

        // ── Succession-patch dispatch (`docs/outcomes/
        // 20260906-scd2-keyed-succession/phases/05b-plan.md`) ───────────────
        // A `refresh: incremental` model with no declared/derivable grain
        // (`metadata.resolved_grain() == None`) is the keyed-succession
        // grain's own undeclared-admission shape (`docs/specs/
        // incremental_shapes.md` §"The succession grain") — dispatched here,
        // before the ordinary `plan.incremental` match, since it carries no
        // `Grain::Key`/`Grain::Partition` plan at all.
        // `resolve_live_succession_cell` itself refuses (`Ok(None)`) for a
        // non-incremental model, a `NotSuccession` classifier verdict, or a
        // state-downgraded cell (technique no longer `SuccessionPatch`), so
        // this guard only needs the grain check.
        if let Some(metadata) = plan
            .model_file
            .metadata
            .as_deref()
            .filter(|m| m.resolved_grain().is_none())
        {
            let db_table_name = plan.model_file.db_name_owned();
            let clean_sql = smelt_parser::strip_frontmatter(&plan.sql).to_string();
            let (succession_source_facts, succession_explicitly_mutable) =
                build_maint_source_facts(&plan.model_file, source_infos);
            let succession_source_refs =
                crate::maintenance_driver::succession::build_succession_source_refs(
                    &plan.model_file,
                    source_infos,
                );
            if let Some(cell) = crate::maintenance_driver::succession::resolve_live_succession_cell(
                &clean_sql,
                &db_table_name,
                metadata,
                &succession_source_facts,
                &succession_explicitly_mutable,
                &succession_source_refs,
                &availability,
                schema,
                model_target,
                source_infos,
            )? {
                let (Some(s), Some(e)) = (start_date, end_date) else {
                    anyhow::bail!(
                        "Model '{}' derives the succession grain (`docs/specs/\
                         incremental_shapes.md` §\"The succession grain\") and was run with no \
                         event-time window — both --event-time-start/--event-time-end are \
                         required.",
                        plan.name
                    );
                };
                let steps = crate::maintenance_driver::driving_steps(
                    &s.format("%Y-%m-%d").to_string(),
                    &e.format("%Y-%m-%d").to_string(),
                    &cell.granularity,
                )?;
                let columns: Vec<(String, smelt_types::DataType)> = upstream_schemas_for_bootstrap
                    .models
                    .get(&plan.model_file.name)
                    .map(|cols| {
                        cols.iter()
                            .map(|(name, typed)| (name.clone(), typed.data_type.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let succession_retry_policy =
                    RetryPolicy::from_request(request, run_id, &plan.name, reporter);
                let succession_probe_policy =
                    probe_policy_for_model(config, prior_runs, &plan.name);
                let succession_result =
                    crate::maintenance_driver::succession::execute_succession_maintenance(
                        backend,
                        &plan.name,
                        schema,
                        &db_table_name,
                        &steps,
                        &cell,
                        &columns,
                        &succession_retry_policy,
                        &succession_probe_policy,
                        reporter,
                        run_id,
                    )
                    .await?;
                manifest_entries.insert(
                    plan.name.clone(),
                    ModelRunRecord {
                        strategy: "succession_patch".to_string(),
                        time_range: Some(TimeRangeRecord {
                            start: s.format("%Y-%m-%d").to_string(),
                            end: e.format("%Y-%m-%d").to_string(),
                        }),
                        partitions_updated: vec![],
                        row_count: succession_result.row_count,
                        duration_ms: model_start.elapsed().as_millis() as u64,
                        batch_safety: Some("succession".to_string()),
                        outcome: smelt_state::RunOutcomeKind::Success,
                        definition_hash: compute_model_hash(&plan.sql),
                        error: None,
                        retry_count: 0,
                        probes: Vec::new(),
                        subsumed: None,
                        deferred_cells: Vec::new(),
                    },
                );
                reporter.model_completed(
                    run_id,
                    &plan.name,
                    succession_result.row_count,
                    model_start.elapsed(),
                );
                if request.run_checks {
                    let (outcomes, to_skip) = run_model_checks(
                        &plan.name,
                        checks_by_model,
                        compilers,
                        backends,
                        target_assignments,
                        ephemeral_resolvers,
                        config.as_ref(),
                        upstream_map,
                        selected,
                        reporter,
                        run_id,
                    )
                    .await;
                    check_results.extend(outcomes);
                    skip_set.extend(to_skip);
                }
                return Ok(ModelOutcome::Completed(ModelSuccess {
                    manifest_entries,
                    check_results,
                    skip_set,
                    rows: succession_result.row_count,
                }));
            }
        }

        let result: Result<()> = match plan.incremental.as_ref().filter(|_| !force_full_refresh) {
            Some(inc_plan) => {
                let backend_default_strategy = backend.resolve_strategy(&inc_plan.config);

                // Build source bound map once per model for source-filter pushdown (BUG-073).
                // The model SQL is the same for every batch — compute once and reuse.
                // `source_timeseries` is the project-wide smelt-ref → TimeseriesConfig map
                // (built from model frontmatter + source YAML declarations in Phase 2), so it
                // also contains this model's *own* frontmatter entry (every batched model
                // declares `timeseries:` on itself). Restrict `dep_ts` to the model's actual
                // upstream refs (`model.refs`) — otherwise the self-entry would inflate
                // `per_model_source_bounds` with a spurious zero-margin entry for a ref that
                // never appears in the model's own SQL, breaking the single-source B0
                // transparent-slice classification (`is_transparent_single_source`) for every
                // model that happens to declare `timeseries:` on itself (i.e. every batched
                // model).
                let model_ref_paths: std::collections::HashSet<String> = plan
                    .model_file
                    .refs
                    .iter()
                    .map(|r| format!("smelt.{}", r.smelt_ref.to_path().join(".")))
                    .collect();
                let sql_for_bounds = smelt_parser::strip_frontmatter(&plan.sql);

                // MP11 (`incremental_models.md` §"Per-cell admission"): read the
                // creation trigger's write strategy off the derived
                // `MaintenancePlan` rather than a hardcoded constant.
                // `SourceFacts` are assembled with the same bare-name
                // convention `smelt-db::maintenance_plan`'s Salsa query uses
                // (`SourceFacts::name` strips the `sources.` breadcrumb) so
                // trigger names agree with `derive_column_groups`'
                // `mutation_sensitivity` keys. `allow_full_scan: true` is
                // safe here regardless of the model's real
                // `maintenance.scan_bounds` declaration: the creation
                // trigger's `Grain::Partition` arm (`derive_new_data`) always
                // admits `Technique::DeleteInsert` unconditionally — no
                // admission check reads `allow_full_scan` on that path — so
                // this can't spuriously widen what actually executes.
                let maint_source_facts: Vec<smelt_logical::maintenance::SourceFacts> = plan
                    .model_file
                    .refs
                    .iter()
                    .filter_map(|r| {
                        let segs = r.smelt_ref.to_path();
                        let info = source_infos.iter().find(|s| s.address_segments == segs)?;
                        // Bare name: strip only a leading `sources` breadcrumb
                        // (matching `smelt-db::maintenance_plan`'s
                        // `stripped.strip_prefix("sources.")` exactly) — NOT
                        // just the last segment, which would collapse a
                        // multi-level address like `sources.raw.users` down
                        // to `users` and disagree with `SourceFacts::name`
                        // elsewhere (`scan_bounds.per_source` keys,
                        // `derive_column_groups`'s `mutation_sensitivity`).
                        let bare = match segs.split_first() {
                            Some((first, rest)) if first == "sources" => rest.join("."),
                            _ => segs.join("."),
                        };
                        Some(smelt_db::queries::maintenance::source_facts(
                            &bare,
                            Some(info),
                            true,
                        ))
                    })
                    .collect();
                // `explicitly_mutable` names sources whose OWN source YAML
                // declares `mutation_profile: mutable_snapshot` — checked
                // against `source_infos`' raw `mutation_profile` field
                // directly, NOT `maint_source_facts`' already-defaulted
                // `mutation` field above. `source_facts()` fails closed to
                // `MutableSnapshot` for an UNDECLARED source too (the
                // stricter posture for admission purposes elsewhere in this
                // function); filtering on that defaulted field here would
                // treat every undeclared, unclocked source (e.g. a plain
                // `refresh: incremental` model's own append-only-by-default
                // upstream) as "explicitly mutable", spuriously admitting a
                // `Trigger::UpstreamMutation` cell
                // `derive_model_maintenance_plan` never intended to derive
                // for it (see that function's own doc comment: "explicitly
                // declares... not merely the fail-closed default"). Mirrors
                // `crates/smelt-runtime/tests/technique_lowering.rs`'s
                // `real_fixture_examples_timeseries_admits_column_scoped_merge_cell`.
                let explicitly_mutable: std::collections::HashSet<String> = plan
                    .model_file
                    .refs
                    .iter()
                    .filter_map(|r| {
                        let segs = r.smelt_ref.to_path();
                        let info = source_infos.iter().find(|s| s.address_segments == segs)?;
                        let mutable = info.mutation_profile.as_ref().is_some_and(|m| {
                            m.kind == smelt_core::sources::MutationProfile::Mutable
                        });
                        if !mutable {
                            return None;
                        }
                        let bare = match segs.split_first() {
                            Some((first, rest)) if first == "sources" => rest.join("."),
                            _ => segs.join("."),
                        };
                        Some(bare)
                    })
                    .collect();
                // T3 (`docs/plans/20260715-composed-axes-conditional-
                // maintenance.md` Phase E3): the model-edge-sourced creation
                // cell's delta-restriction facts, resolved once per model
                // (not per batch — they don't vary across this model's own
                // batches). `model_edges_for` mirrors `crate::propagation::
                // derive_clamp_and_locality`'s own edge extraction; `None`
                // when this model reads no maintained-model upstream, the
                // plan derives no creation cell for the driving edge, or the
                // model's own row identity is not a single column — the
                // batch loop below then always takes the ordinary widened
                // scan, unchanged from before this phase. Hoisted above
                // `resolved_strategy` below so both consult the same
                // `model_edges` list without computing it twice.
                let model_edges = model_edges_for(&plan.model_file, model_by_addr, source_infos);
                let resolved_strategy = match plan.model_file.metadata.as_deref() {
                    Some(metadata) => crate::maintenance_driver::resolve_incremental_strategy(
                        &sql_for_bounds,
                        &plan.model_file.db_name_owned(),
                        metadata,
                        &maint_source_facts,
                        &explicitly_mutable,
                        &model_edges,
                        backend_default_strategy.clone(),
                        backend.capabilities().supports_column_scoped_merge,
                        &availability,
                    )?,
                    None => backend_default_strategy,
                };

                let delta_restriction_facts = if model_edges.is_empty() {
                    None
                } else {
                    match plan.model_file.metadata.as_deref() {
                        Some(metadata) => {
                            crate::maintenance_driver::resolve_live_delta_restriction_facts(
                                &sql_for_bounds,
                                &plan.model_file.db_name_owned(),
                                metadata,
                                &maint_source_facts,
                                &explicitly_mutable,
                                &model_edges,
                                &availability,
                            )
                            .map_err(|e| anyhow::anyhow!("{}", e))?
                        }
                        None => None,
                    }
                };
                // 27e (`docs/outcomes/20260815-definition-delta-migrate/
                // phases/27e-plan.md`): when this model reads NO maintained-
                // model upstream (`model_edges` empty — the model-edge route
                // above never resolves), it may still be driven by an
                // external `mutable_snapshot` source with no native change
                // feed. `resolve_live_external_delta_restriction_facts`
                // consults the SAME derived plan's `UpstreamMutation` cell
                // for that source; `None` in every case where the model-edge
                // route would also fall back — the caller's safe default
                // stays the ordinary widened scan.
                let external_delta_restriction_facts = if !model_edges.is_empty() {
                    None
                } else {
                    match plan.model_file.metadata.as_deref() {
                        Some(metadata) => {
                            // Real declared `referential_integrity:` facts
                            // (`sources.md` §"Referential integrity") — keyed
                            // the same bare-address way `maint_source_facts`
                            // above is, so the `DeclaredReferentialIntegrity`
                            // P1 route is actually reachable live, not only
                            // through a hand-built fixture.
                            let source_ri: smelt_logical::maintenance::derive::SourceReferentialIntegrity =
                                source_infos
                                    .iter()
                                    .filter_map(|info| {
                                        let ri = info.referential_integrity.clone()?;
                                        let segs = &info.address_segments;
                                        let bare = match segs.split_first() {
                                            Some((first, rest)) if first == "sources" => {
                                                rest.join(".")
                                            }
                                            _ => segs.join("."),
                                        };
                                        Some((bare, ri))
                                    })
                                    .collect();
                            crate::maintenance_driver::resolve_live_external_delta_restriction_facts(
                                &sql_for_bounds,
                                &plan.model_file.db_name_owned(),
                                metadata,
                                &maint_source_facts,
                                &explicitly_mutable,
                                &source_ri,
                                backend.capabilities().supports_fingerprint_sidecar,
                                &availability,
                            )
                            .map_err(|e| anyhow::anyhow!("{}", e))?
                        }
                        None => None,
                    }
                };
                // Live dispatch is licensed only for a DuckDB target running
                // the creation trigger's `DeleteInsert` strategy — the same
                // scoping the existing report-and-execute branch below
                // already uses (`read_observed_delta_changed_keys` is
                // DuckDB-only, and `execute_delete_insert_with_delta_
                // restriction` calls `Backend::execute_statement_group`
                // directly rather than `Backend::execute_model_incremental`,
                // so a non-DuckDB backend's own `delete_and_insert_
                // transactional` override — e.g. Spark's — must stay
                // reachable unchanged).
                let use_delta_restricted_dispatch = delta_restriction_facts.is_some()
                    && backend.dialect() == smelt_backend::SqlDialect::DuckDB
                    && matches!(
                        resolved_strategy,
                        smelt_backend::IncrementalStrategy::DeleteInsert
                    );
                let use_external_delta_restricted_dispatch = external_delta_restriction_facts
                    .is_some()
                    && backend.dialect() == smelt_backend::SqlDialect::DuckDB
                    && matches!(
                        resolved_strategy,
                        smelt_backend::IncrementalStrategy::DeleteInsert
                    );

                // MP11 (`incremental_models.md` §"Per-cell admission"): consult
                // the SAME derived `MaintenancePlan` for a live
                // `ColumnScopedMerge` cell on one of the model's
                // explicitly-mutable dimension sources. When one resolves
                // live, the batch loop below dispatches to a column-scoped
                // `MERGE` instead of the default region-recompute path — "the
                // driver loop becomes the per-cell technique executor"
                // (`docs/plans/20260707-maintenance-plan-impl.md` Phase
                // MP11). Deciding WHETHER a mutation actually happened this
                // run (forward propagation / scheduling) is MP15's job; this
                // only asks which technique the plan admits for the trigger,
                // exactly like `resolve_incremental_strategy` above does for
                // the creation trigger.
                let column_scoped_cell = match plan.model_file.metadata.as_deref() {
                    Some(metadata) => crate::maintenance_driver::resolve_live_column_scoped_cell(
                        &sql_for_bounds,
                        &plan.model_file.db_name_owned(),
                        metadata,
                        &maint_source_facts,
                        &explicitly_mutable,
                        backend.capabilities().supports_column_scoped_merge,
                        &request.technique_overrides,
                        &availability,
                    )?,
                    None => None,
                };

                // The keyless (`RowIdentity::WholeRow`) counterpart of
                // `column_scoped_cell` above (`docs/outcomes/
                // 20260815-definition-delta-migrate/phases/27c-plan.md`): a
                // `grain: partition` output has no `unique_key`, so a
                // membership-sensitive `UpstreamMutation` cell can never
                // resolve `Technique::ColumnScopedMerge` (that family needs a
                // proven row identity too) — before this phase it fell
                // through to the ordinary unconditional widened-scan
                // DELETE+INSERT every run. `resolve_live_membership_
                // recompute_cell` derives both the keyed (`StagedRecompute`/
                // `DiffPatch`, the keyed run loop's own concern) and keyless
                // (`StagedKeyless`) arms from the SAME plan; only the keyless
                // arm is consumed here — a `RowIdentity::Key` result stays
                // out of scope for this (non-keyed) branch, unchanged from
                // before this phase.
                let membership_recompute_keyless_cell = match plan.model_file.metadata.as_deref()
                {
                    Some(metadata) => {
                        crate::maintenance_driver::resolve_live_membership_recompute_cell(
                            &sql_for_bounds,
                            &plan.model_file.db_name_owned(),
                            metadata,
                            &maint_source_facts,
                            &explicitly_mutable,
                            &request.technique_overrides,
                            &availability,
                        )?
                        .filter(|(_, cell, _, write)| {
                            matches!(
                                cell.row_identity.identity,
                                smelt_logical::maintenance::RowIdentity::WholeRow
                            ) && matches!(
                                write,
                                crate::maintenance_driver::MembershipRecomputeWrite::StagedKeyless {
                                    ..
                                }
                            )
                        })
                    }
                    None => None,
                };

                let dep_ts: std::collections::HashMap<String, (Vec<String>, String)> =
                    source_timeseries
                        .iter()
                        .filter(|(smelt_ref, _)| model_ref_paths.contains(*smelt_ref))
                        .filter_map(|(smelt_ref, ts)| {
                            // Strip the leading "smelt." prefix to get the path segments.
                            let path = smelt_ref.strip_prefix("smelt.")?;
                            let segs: Vec<String> = path.split('.').map(String::from).collect();
                            Some((smelt_ref.clone(), (segs, ts.partition_column.clone())))
                        })
                        .collect();
                let horizon_ceiling = plan
                    .model_file
                    .metadata
                    .as_ref()
                    .and_then(|m| m.horizon_ceiling.as_ref());
                let (per_model_source_bounds, horizon_warnings) =
                    build_source_bound_map(&sql_for_bounds, &dep_ts, horizon_ceiling);
                for warning in &horizon_warnings {
                    warn!("model '{}': {warning}", plan.name);
                }

                let mut used_column_scoped_merge = false;

                // Which physical corner (if any) THIS run's batches should
                // dispatch through instead of the default
                // `execute_model_incremental` (DELETE+INSERT) call — decided
                // once per run, then applied per batch below so the WRITE
                // stays scoped to the SAME `[start, end)` window a plain
                // incremental run would already touch.
                //
                // `Corner::ColumnMerge` is "full-input read, targeted write";
                // `derive_model_maintenance_plan` derives two distinct shapes
                // of it for an `UpstreamMutation` trigger
                // (`incremental_models.md` §"Per-cell admission"):
                // - `PartitionLocal::No` (accepted full scan, the operator
                //   declared `allow_full_scan`): no horizon to clamp the
                //   WRITE to, so it stays targeted by the run's own batch
                //   window plus the `unique_key` MERGE semantics —
                //   `execute_column_scoped_merge_full`.
                // - `PartitionLocal::Yes` (a genuine derived `ScanClamp`):
                //   the horizon-clamped corner F15 was built for —
                //   `execute_column_scoped_merge`/`dimension_horizon_merge`
                //   additionally clamp the dimension batch to
                //   `[conv_ts − H, conv_ts]` on the model's own partition
                //   axis, licensed only when the mutated dimension's join
                //   contribution is provably monotone
                //   (`maintenance_driver::dimension_join_contribution`).
                //
                // A forward-only advance whose window never revisits an
                // already-processed partition still leaves that partition
                // exactly as untouched as the default DELETE+INSERT path
                // would have left it; only the technique used to write the
                // requested window differs.
                let column_merge_dispatch: Option<crate::maintenance_driver::ColumnMergeDispatch> =
                    match column_scoped_cell.as_ref() {
                        Some((source, cell, _suppression)) => {
                            let table_exists = backend
                                .table_exists(schema, &plan.model_file.db_name_owned())
                                .await
                                .unwrap_or(false);
                            // The join-contribution proof is only meaningful
                            // (and only computed) for the `PartitionLocal::Yes`
                            // corner — the accepted-full-scan corner has no
                            // such precondition.
                            let contribution = if matches!(
                                cell.partition_local,
                                smelt_logical::maintenance::PartitionLocal::Yes
                            ) {
                                // The mutated dimension's own declared
                                // `unique_key` (`sources.md` §"Row identity")
                                // — never `SourceFacts`' always-empty
                                // `unique_key` field (`smelt-db`'s
                                // `source_facts()` does not populate it
                                // yet), read straight off the
                                // already-resolved `source_infos`.
                                let dimension_unique_key: Vec<String> = source_infos
                                    .iter()
                                    .find(|info| {
                                        let segs = &info.address_segments;
                                        let bare = match segs.split_first() {
                                            Some((first, rest)) if first == "sources" => {
                                                rest.join(".")
                                            }
                                            _ => segs.join("."),
                                        };
                                        &bare == source
                                    })
                                    .and_then(|info| info.unique_key.clone())
                                    .unwrap_or_default();
                                crate::maintenance_driver::dimension_join_contribution(
                                    &sql_for_bounds,
                                    source,
                                    &dimension_unique_key,
                                )
                            } else {
                                smelt_logical::analysis::join_shape::ContributionVerdict::Monotone
                            };
                            crate::maintenance_driver::decide_column_merge_dispatch(
                                cell,
                                source,
                                table_exists,
                                !inc_plan.config.unique_key.is_empty(),
                                &contribution,
                            )
                        }
                        None => None,
                    };

                // Mutation-happened discrimination
                // (`docs/specs/incremental_models.md` §"When a mutation
                // cell dispatches"): resolved ONCE for the whole run
                // (matching `column_merge_dispatch`'s own once-per-run
                // decision above) — the source's fingerprint does not
                // change across this run's own batches. A `NoOp` verdict
                // overrides `column_merge_dispatch` to `None`, so every
                // batch below falls through to its ordinary DELETE+INSERT
                // path exactly as if no live cell had resolved.
                let mutation_gate = match column_scoped_cell.as_ref() {
                    Some((source, _cell, _suppression)) => {
                        resolve_upstream_mutation_gate(
                            backend,
                            &plan.name,
                            source_infos,
                            source,
                            model_target,
                            schema,
                            file_store,
                            state_io_lock,
                        )
                        .await?
                    }
                    None => None,
                };
                let column_merge_dispatch = if matches!(
                    mutation_gate.as_ref().map(|(v, _)| v),
                    Some(crate::mutation_probe::MutationVerdict::NoOp)
                ) {
                    None
                } else {
                    column_merge_dispatch
                };

                // A key-addressed model-edge cell (`docs/specs/incremental_models.md`
                // §"Upstream model edges") has no run-window axis of its own —
                // its bounded read is the upstream's own affected key set, not
                // a `[start, end)` interval — so it is resolved and (if live)
                // dispatched HERE, before this branch's own self-ref bootstrap
                // or its per-batch DELETE+INSERT loop, and takes this run in
                // place of either regardless of this downstream's own declared
                // `grain:`. `table_exists_before_run` is captured before any
                // write this run performs, INCLUDING the self-ref bootstrap
                // below (which may itself create the table on a first run) —
                // never on the creation run, mirroring the keyed branch's own
                // capture (`docs/outcomes/20260815-definition-delta-migrate/
                // phases/11-plan.md`).
                let table_exists_before_run = backend
                    .table_exists(schema, &plan.model_file.db_name_owned())
                    .await
                    .unwrap_or(false);
                let key_edge_dispatch = resolve_and_dispatch_key_addressed_edge_cell(
                    backend,
                    schema,
                    &plan.name,
                    &plan.model_file,
                    &sql_for_bounds,
                    &plan.model_file.db_name_owned(),
                    &maint_source_facts,
                    &explicitly_mutable,
                    &model_edges,
                    table_exists_before_run,
                    model_by_addr,
                    config,
                    request,
                    compilers.get(model_target),
                    &ephemeral_resolvers[model_target],
                    run_id,
                    reporter,
                    &availability,
                )
                .await?;
                // Mutual exclusion with `column_scoped_cell`/`delta_restriction_
                // facts` above: a key-addressed cell is keyed on the upstream
                // MODEL's bare name, the others on declared SOURCES, so the
                // resolvers read disjoint trigger-name spaces and can never
                // contend for the same trigger by construction — bail loudly
                // rather than silently preferring one if that invariant is
                // ever violated.
                if let (Some(dispatch), Some((source, _, _))) =
                    (key_edge_dispatch.as_ref(), column_scoped_cell.as_ref())
                {
                    if *source == dispatch.edge_name {
                        anyhow::bail!(
                            "internal inconsistency: model '{}' resolved BOTH a key-addressed \
                             model-edge cell and a column-scoped-merge cell for the same trigger \
                             name '{}' — these must be disjoint (model edges vs declared \
                             sources)",
                            plan.name,
                            dispatch.edge_name
                        );
                    }
                }

                'run_dispatch_or_batches: {
                if let Some(dispatch) = key_edge_dispatch {
                    let strategy = if dispatch.used_diff_patch {
                        "diff_patch".to_string()
                    } else {
                        "per_group_recompute".to_string()
                    };
                    total_rows = dispatch.result.row_count;
                    manifest_entries.insert(
                        plan.name.clone(),
                        ModelRunRecord {
                            strategy,
                            time_range: None,
                            partitions_updated: vec![],
                            row_count: total_rows,
                            duration_ms: model_start.elapsed().as_millis() as u64,
                            batch_safety: None,
                            outcome: smelt_state::RunOutcomeKind::Success,
                            definition_hash: compute_model_hash(&plan.sql),
                            error: None,
                            retry_count: sink.retry_count(),
                            probes: Vec::new(),
                            subsumed: None,
                            deferred_cells: Vec::new(),
                        },
                    );
                    break 'run_dispatch_or_batches;
                }

                // The keyless membership-recompute dispatch
                // (`membership_recompute_keyless_cell` above): a live
                // `RowIdentity::WholeRow` `UpstreamMutation` cell replaces
                // the whole run — never dispatched on the creation run
                // (`table_exists_before_run`), and never when the mutated
                // source's own fingerprint shows no real change since the
                // last recorded baseline (mirrors `column_scoped_cell`'s own
                // `mutation_gate` posture above).
                if let Some((source, _cell, _group_columns, write)) =
                    membership_recompute_keyless_cell.as_ref()
                {
                    if table_exists_before_run {
                        let mutation_gate = resolve_upstream_mutation_gate(
                            backend,
                            &plan.name,
                            source_infos,
                            source,
                            model_target,
                            schema,
                            file_store,
                            state_io_lock,
                        )
                        .await?;
                        let mutation_should_dispatch = !matches!(
                            mutation_gate.as_ref().map(|(v, _)| v),
                            Some(crate::mutation_probe::MutationVerdict::NoOp)
                        );
                        if mutation_should_dispatch {
                            let crate::maintenance_driver::MembershipRecomputeWrite::StagedKeyless {
                                compared_columns: _,
                            } = write
                            else {
                                unreachable!(
                                    "membership_recompute_keyless_cell is filtered to \
                                     StagedKeyless only"
                                );
                            };
                            let compiler = compilers.get(model_target);
                            let resolver = &ephemeral_resolvers[model_target];
                            let compiled = compiler.compile_with_sql_and_ephemerals(
                                &plan.model_file,
                                schema,
                                &sql_for_bounds,
                                resolver,
                            )?;
                            let retry_policy =
                                RetryPolicy::from_request(request, run_id, &plan.name, reporter);
                            let result =
                                crate::maintenance_driver::execute_staged_keyless_recompute(
                                    backend,
                                    schema,
                                    &plan.model_file.db_name_owned(),
                                    &compiled.sql,
                                    &retry_policy,
                                )
                                .await
                                .map_err(|e| anyhow::anyhow!("{}", e))?;
                            total_rows = result.row_count;
                            record_upstream_mutation_baseline(
                                mutation_gate,
                                source,
                                file_store,
                                state_io_lock,
                            )
                            .await;
                            manifest_entries.insert(
                                plan.name.clone(),
                                ModelRunRecord {
                                    strategy: "delete_insert_suppressed".to_string(),
                                    time_range: None,
                                    partitions_updated: vec![],
                                    row_count: total_rows,
                                    duration_ms: model_start.elapsed().as_millis() as u64,
                                    batch_safety: None,
                                    outcome: smelt_state::RunOutcomeKind::Success,
                                    definition_hash: compute_model_hash(&plan.sql),
                                    error: None,
                                    retry_count: sink.retry_count(),
                                    probes: Vec::new(),
                                    subsumed: None,
                                    deferred_cells: Vec::new(),
                                },
                            );
                            break 'run_dispatch_or_batches;
                        }
                    }
                }

                // First-run bootstrap for a self-referential model
                // (`docs/specs/incremental_shapes.md` §"First-run and backfill"
                // — "First-run bootstrap for a self-referential model"):
                // when the target doesn't exist yet, `CREATE TABLE … AS
                // SELECT …` over the first batch cannot resolve the
                // self-reference (no engine can create a table and read it
                // in the same statement). Materialise an EMPTY target
                // carrying the model's own resolved output schema first
                // (`bootstrap_self_ref_empty_target`), then fall through to
                // the ordinary per-batch DELETE+INSERT loop below, whose
                // first iteration now sees an existing-but-empty table and
                // reads no prior state, exactly as if the table had been
                // seeded by hand with zero rows.
                if crate::compile::is_self_referential(&plan.model_file) {
                    let table_exists = backend
                        .table_exists(schema, &plan.model_file.db_name_owned())
                        .await?;
                    if !table_exists {
                        bootstrap_self_ref_empty_target(
                            request,
                            backend,
                            schema,
                            &plan.model_file,
                            &plan.name,
                            upstream_schemas_for_bootstrap,
                            reporter,
                            run_id,
                        )
                        .await?;
                    }
                }

                // Accumulates every probe's held/skipped outcome across this
                // model's batches, for `ModelRunRecord.probes`
                // (`docs/specs/run_state.md` §"Run manifest"). The batch
                // loop is sequential, so a plain `mut` Vec is sound — no
                // concurrent writer.
                let mut model_probe_records: Vec<smelt_state::ProbeRecord> = Vec::new();

                for (batch_idx, batch) in inc_plan.batches.iter().enumerate() {
                    if cancel.is_cancelled() {
                        return Ok(ModelOutcome::Cancelled);
                    }

                    let batch_start_time = Instant::now();

                    let clean_sql = smelt_parser::strip_frontmatter(&plan.sql);

                    // Source-filter pushdown: narrow each source read to this batch's
                    // window (partition_start / partition_end) plus per-source bounds
                    // derived from the model SQL's INTERVAL patterns. `batch.partition_start
                    // / partition_end` is the **derived output window**, not the CLI-declared
                    // run window verbatim (`windowing::compute_incremental_windows`
                    // widens the declared window by the model's own partition-column
                    // skew before chunking, `docs/specs/model_transforms.md` §Semantics
                    // "The output window is derived, never assumed") — a skewed model's
                    // batch here may already reach outside what the user typed on the
                    // command line. Source filters derive from this (already-derived)
                    // window so the source scan tracks the partition being produced,
                    // not the potentially wider DELETE range.
                    let run_range = TimeRange {
                        start: batch.partition_start.to_string(),
                        end: batch.partition_end.to_string(),
                        axis: smelt_logical::PartitionAxis::Calendar,
                    };

                    // Two-layer widened-scan + exact output clamp
                    // (`docs/specs/model_transforms.md` §Semantics — "Source-filter
                    // pushdown + the two clamps"): the *scan* may read a margin
                    // (handled per-source by `inject_source_filters`, which widens
                    // each bounded source independently), but the *output clamp*
                    // must equal the output window exactly — the margin is read but
                    // never re-written. B0 (unified pushdown-depth walk,
                    // `docs/research/20260703-model-updates.md` §3.3/§3.5): for the
                    // transparent slice — a single bounded source with no lookback
                    // margin AND zero partition-column skew — the source-level filter
                    // on the exact output-window batch *is* the output clamp; the
                    // outer `inject_time_filter` wrap would inject a textually
                    // identical, redundant filter. Skip it and rely solely on the
                    // source-level filter (`derive_batch_filtered_sql`'s
                    // `is_transparent_single_source(...) && skew == Skew::ZERO` gate).
                    // A model with a real lookback margin, a genuine partition-column
                    // skew, or more than one source keeps both layers, but the outer
                    // clamp uses the narrow output-window batch (`run_range`), not the
                    // widened scan window — the write window must equal the output
                    // window.
                    // Two-layer widened-scan + exact output clamp, then
                    // compile-time clock pinning — the output clamp ranges over
                    // the declared `partition_column` (the same output-axis
                    // column the DELETE below ranges over), and every batch of
                    // this run shares the one `run_start` literal. Shared with
                    // the `--dry-run` statement-emission branch via
                    // `derive_batch_filtered_sql` so a dry-run derives a batch's
                    // SQL exactly as this live run does.
                    let filtered_sql = derive_batch_filtered_sql(
                        &clean_sql,
                        &inc_plan.timeseries.partition_column,
                        &per_model_source_bounds,
                        &run_range,
                        run_start,
                        inc_plan.skew,
                    )?;

                    let compiler = compilers.get(model_target);
                    let resolver = &ephemeral_resolvers[model_target];
                    let compiled = compiler.compile_with_sql_and_ephemerals(
                        &plan.model_file,
                        schema,
                        &filtered_sql,
                        resolver,
                    )?;
                    reporter.model_compiled(run_id, &plan.name, &compiled.sql);

                    // Live dispatch of the declared model-scoped probes
                    // before this batch's write — same obligation as the
                    // full-refresh site above, scoped to this batch's own
                    // compiled (filtered) SQL
                    // (`docs/specs/model_properties.md` §"Probe obligation").
                    let declared_probes = crate::model_probes::declared_model_probes(
                        &plan.name,
                        &format!(
                            "{}.{} batch [{}, {})",
                            schema,
                            plan.model_file.db_name_owned(),
                            batch.partition_start,
                            batch.partition_end,
                        ),
                        plan.model_file.metadata.as_deref(),
                        Some(&inc_plan.timeseries),
                        &compiled.sql,
                        smelt_backend::maintenance_dialect(backend.dialect()),
                    );
                    model_probe_records.extend(
                        crate::model_probes::dispatch_declared_model_probes(
                            backend,
                            &probe_policy_for_model(config, prior_runs, &plan.name),
                            &declared_probes,
                        )
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?,
                    );

                    // Live dispatch of the source append-only posture probe
                    // (`docs/specs/model_properties.md` §"Probe obligation",
                    // row `mutation_profile.kind: append_only`) before this
                    // batch's write — same pre-write obligation as the
                    // model-scoped probes above, but scoped to the model's
                    // consumed sources' recorded per-partition baselines
                    // rather than this run's own compiled SQL. A held probe
                    // refreshes the recorded baseline; a violation fails the
                    // run before the write.
                    {
                        let _io_guard = state_io_lock.lock().await;
                        let source_postures = file_store
                            .load_source_postures()
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        let source_probes = crate::source_probes::append_only_posture_probes(
                            &plan.name,
                            &format!(
                                "{}.{} batch [{}, {})",
                                schema,
                                plan.model_file.db_name_owned(),
                                batch.partition_start,
                                batch.partition_end,
                            ),
                            &plan.model_file,
                            source_infos,
                            &source_postures,
                            model_target,
                            schema,
                            smelt_backend::maintenance_dialect(backend.dialect()),
                        );
                        if !source_probes.is_empty() {
                            let (refreshed, records) =
                                crate::source_probes::dispatch_and_record_append_only_postures(
                                    backend,
                                    &probe_policy_for_model(config, prior_runs, &plan.name),
                                    &source_probes,
                                )
                                .await
                                .map_err(|e| anyhow::anyhow!("{}", e))?;
                            model_probe_records.extend(records);
                            if !refreshed.is_empty() {
                                let mut source_postures = source_postures;
                                for r in refreshed {
                                    source_postures.record(&r.source_address, r.partitions);
                                }
                                let _ = file_store.save_source_postures(&source_postures);
                            }
                        }
                    }

                    // Live dispatch of the contract-lattice `frozen_horizon`
                    // late-arrival probe (`docs/specs/incremental_models.md`
                    // §"The contract lattice") before this batch's write —
                    // opt-in only (empty probe set absent a `contract.
                    // frozen_horizon` declaration), scoped to the model's
                    // clocked sources' recorded frozen-band baselines. The
                    // baseline is refreshed whether the probe held OR fired,
                    // so a genuine late arrival is reported once, not every
                    // subsequent run
                    // (`docs/outcomes/20260809-contract-lattice-v1/phases/
                    // 03-plan.md`).
                    if let Some(end_date) = end_date {
                        let _io_guard = state_io_lock.lock().await;
                        let frozen_band_baselines = file_store
                            .load_frozen_band_baselines()
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        let contract_probes = crate::contract_probes::frozen_horizon_probes(
                            &plan.name,
                            &format!(
                                "{}.{} batch [{}, {})",
                                schema,
                                plan.model_file.db_name_owned(),
                                batch.partition_start,
                                batch.partition_end,
                            ),
                            &plan.model_file,
                            plan.model_file.metadata.as_deref(),
                            source_infos,
                            end_date,
                            model_target,
                            schema,
                            smelt_backend::maintenance_dialect(backend.dialect()),
                        );
                        if !contract_probes.is_empty() {
                            let result =
                                crate::contract_probes::dispatch_and_record_frozen_horizon_probes(
                                    backend,
                                    &probe_policy_for_model(config, prior_runs, &plan.name),
                                    &contract_probes,
                                    &frozen_band_baselines,
                                )
                                .await
                                .map_err(|e| anyhow::anyhow!("{}", e))?;
                            model_probe_records.extend(result.records);
                            if !result.refreshed.is_empty() {
                                let mut frozen_band_baselines = frozen_band_baselines;
                                for r in result.refreshed {
                                    frozen_band_baselines.record(&r.source_address, r.partitions);
                                }
                                let _ =
                                    file_store.save_frozen_band_baselines(&frozen_band_baselines);
                            }
                            if let Some(violation) = result.violations.first() {
                                return Err(anyhow::anyhow!("{}", violation.message));
                            }
                        }
                    }

                    // Live dispatch of the contract-lattice `deferral` probe
                    // (`docs/specs/incremental_models.md` §"The contract
                    // lattice") before this batch's write — opt-in only
                    // (empty probe set absent a `contract.deferral`
                    // declaration). Unlike `frozen_horizon`'s probe, this
                    // one emits no SQL: both frontiers it compares are
                    // already-recorded ledger state
                    // (`IntervalStore`/`LandedDeltaStore`), so it only reads
                    // state the run already writes elsewhere, under the same
                    // `state_io_lock` critical section.
                    {
                        let clocked_source_addresses: Vec<String> = maint_source_facts
                            .iter()
                            .filter(|sf| {
                                sf.partition_col.is_some()
                                    && sf.mutation
                                        == smelt_logical::maintenance::MutationProfile::AppendOnly
                            })
                            .map(|sf| sf.name.clone())
                            .collect();
                        let deferral_probes = crate::contract_probes::deferral_probes(
                            &plan.name,
                            &format!(
                                "{}.{} batch [{}, {})",
                                schema,
                                plan.model_file.db_name_owned(),
                                batch.partition_start,
                                batch.partition_end,
                            ),
                            plan.model_file.metadata.as_deref(),
                        );
                        if !deferral_probes.is_empty() {
                            let _io_guard = state_io_lock.lock().await;
                            let interval_store = file_store
                                .load_intervals()
                                .map_err(|e| anyhow::anyhow!("{}", e))?;
                            let landed_deltas = file_store
                                .load_landed_deltas()
                                .map_err(|e| anyhow::anyhow!("{}", e))?;
                            let (records, violations) = crate::contract_probes::evaluate_deferral(
                                &probe_policy_for_model(config, prior_runs, &plan.name),
                                &deferral_probes,
                                &plan.name,
                                &clocked_source_addresses,
                                &interval_store,
                                &landed_deltas,
                            );
                            model_probe_records.extend(records);
                            if let Some(violation) = violations.first() {
                                return Err(anyhow::anyhow!("{}", violation.message));
                            }
                        }
                    }

                    // The DELETE range must equal exactly what the INSERT writes —
                    // the write window equals the output window
                    // (`docs/specs/model_transforms.md` §Constraints — "Write window
                    // = output window; scan window ⊇ output window"). `inject_time_filter`
                    // clamps the wrapped output on `partition_column` to the narrow
                    // `run_range` (`[partition_start, partition_end)`) — the SAME
                    // column and window this DELETE ranges over, so the two agree
                    // by construction. Widening the DELETE to the scan's margin
                    // would re-delete-and-rewrite the neighboring partition using a
                    // scan sized for *this* batch's margin, not that partition's
                    // own — silently corrupting it.
                    let partition = PartitionRange {
                        column: inc_plan.timeseries.partition_column.clone(),
                        start: batch.partition_start.to_string(),
                        end: batch.partition_end.to_string(),
                        axis: batch.partition_start.axis(),
                    };

                    // Reconciliation ledger (`docs/specs/incremental_models.md`
                    // §"The reconciliation ledger"): every DuckDB DeleteInsert-family
                    // write for this batch — the plain widened-scan recompute, the
                    // model-edge-restricted recompute, or the external-sidecar-
                    // restricted recompute — resets this batch's own
                    // `[partition.start, partition.end)` slice in `_smelt_ledger`
                    // (whole-row group `{*}`, nominal input `self`, watermarked to
                    // the region's own end), run in the SAME backend transaction as
                    // the write it protects via `Backend::execute_write_with_
                    // bookkeeping` (state-residency: `.smelt/reconciliation.json`
                    // no longer exists — this table IS the ledger). Hoisted above
                    // the three DeleteInsert dispatch arms below (model-edge
                    // restricted, external-sidecar restricted, plain) so all three
                    // build the SAME reset from `generate_ledger_recompute_reset_
                    // sqls`, instead of each constructing (or omitting) its own.
                    // Not built when a live `ColumnScopedMerge` cell dispatches
                    // (`column_merge_dispatch.is_some()`) — that technique is not a
                    // region DeleteInsert and its own MP12 ledger interaction is
                    // unrelated to this reset. Whether the ledger is available at
                    // all is now this run's own resolved
                    // `StateAvailability` (`docs/outcomes/20260904-state-residency/
                    // outcome.md` phase 5), not a raw dialect check — on a
                    // ledger-less target this is no longer a silent skip: the
                    // per-cell technique already carries a recorded
                    // `state_downgrade` (`smelt-logical`'s
                    // `resolve_availability`), which `smelt explain` and the
                    // warning diagnostic surface (phase 6). No reporter
                    // event is emitted for the skip anymore — that stand-in
                    // channel is retired.
                    let ledger_reset_sqls = if column_merge_dispatch.is_none()
                        && availability.contains(
                            smelt_logical::maintenance::availability::StateStructure::ReconciliationLedger,
                        )
                    {
                        Some(smelt_state::ddl_duckdb::generate_ledger_recompute_reset_sqls(
                            schema,
                            &plan.name,
                            "{*}",
                            &partition.start,
                            &partition.end,
                            "self",
                            &partition.end,
                        ))
                    } else {
                        if column_merge_dispatch.is_none() {
                            tracing::debug!(
                                model = %plan.name,
                                dialect = backend.dialect().name(),
                                "region-recompute reset skipped: the reconciliation ledger is \
                                 unavailable for this target — the affected cell's own \
                                 recorded state_downgrade is the user-visible channel"
                            );
                        }
                        None
                    };
                    let ledger_ensure_sqls: Vec<String> = if ledger_reset_sqls.is_some() {
                        vec![smelt_state::ddl_duckdb::generate_ledger_table_ddl(schema)]
                    } else {
                        Vec::new()
                    };
                    let ledger_pre_write_sqls: Vec<String> =
                        ledger_reset_sqls.clone().unwrap_or_default();

                    // T3: re-checked per batch (not hoisted with
                    // `use_delta_restricted_dispatch` above) because
                    // `table_exists` genuinely varies across batches of the
                    // SAME model — a non-self-referential model's first
                    // batch finds no target yet (the bootstrap `CREATE
                    // TABLE AS` case below), later batches find one.
                    // `None` here — rather than a fact-implies-Some
                    // `.expect()` below — is what lets the `if let`
                    // dispatch beneath fall through to the ordinary
                    // widened-scan branch with no unwrap at all.
                    let restricted_facts_this_batch: Option<
                        &crate::maintenance_driver::DeltaRestrictionFacts,
                    > = if use_delta_restricted_dispatch {
                        let exists = backend
                            .table_exists(schema, &plan.model_file.db_name_owned())
                            .await
                            .unwrap_or(false);
                        if exists {
                            delta_restriction_facts.as_ref()
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    // Same re-checked-per-batch reasoning as
                    // `restricted_facts_this_batch` above, for the external-
                    // sidecar route.
                    let external_restricted_facts_this_batch: Option<
                        &crate::maintenance_driver::ExternalDeltaRestrictionFacts,
                    > = if use_external_delta_restricted_dispatch {
                        let exists = backend
                            .table_exists(schema, &plan.model_file.db_name_owned())
                            .await
                            .unwrap_or(false);
                        if exists {
                            external_delta_restriction_facts.as_ref()
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let retry_policy =
                        RetryPolicy::from_request(request, run_id, &plan.name, reporter);
                    let exec_result = if let Some(dispatch) = column_merge_dispatch.as_ref() {
                        // MP11 (`incremental_models.md` §"Per-cell admission"):
                        // the live `UpstreamMutation` cell resolved to
                        // `Technique::ColumnScopedMerge` — `compiled.sql` is
                        // ALREADY filtered to this batch's `run_range`
                        // (`inject_time_filter`/`inject_source_filters`
                        // above), so `MERGE`ing it in on `unique_key` keeps
                        // the write scoped to exactly the window a
                        // DELETE+INSERT would have touched, but as a keyed
                        // `MERGE` — the driver loop becoming the per-cell
                        // technique executor.
                        used_column_scoped_merge = true;
                        // Same live cell `column_scoped_cell` resolved above —
                        // its `WriteSuppression` verdict (T1, Phase C4) was
                        // already derived there, from the SAME plan/comparability
                        // read; not re-derived per batch.
                        let suppression = column_scoped_cell
                            .as_ref()
                            .map(|(_, _, s)| s)
                            .expect("dispatch is Some only when column_scoped_cell resolved live");
                        match dispatch {
                            crate::maintenance_driver::ColumnMergeDispatch::Full => {
                                crate::maintenance_driver::execute_column_scoped_merge_full(
                                    backend,
                                    schema,
                                    &plan.model_file.db_name_owned(),
                                    &inc_plan.config.unique_key,
                                    &compiled.sql,
                                    &compiled.output_columns,
                                    suppression,
                                    &partition,
                                    &retry_policy,
                                )
                                .await
                                .map_err(|e| anyhow::anyhow!("{}", e))?
                            }
                            crate::maintenance_driver::ColumnMergeDispatch::Clamped(scan) => {
                                let batch_width_days = batch
                                    .partition_start
                                    .units_between(&batch.partition_end)
                                    .max(0)
                                    as u64;
                                let batch_width =
                                    smelt_logical::analysis::source_bounds::Seconds::days(
                                        batch_width_days,
                                    );
                                let bound = crate::maintenance_driver::widen_horizon_for_batch(
                                    scan,
                                    batch_width,
                                );
                                // The mutated dimension's join contribution
                                // was already proven monotone when
                                // `column_merge_dispatch` was computed above
                                // (`dimension_join_contribution`) — this is
                                // not a second, independent re-derivation.
                                let contribution =
                                    smelt_logical::analysis::join_shape::ContributionVerdict::Monotone;
                                let conv_ts = batch.partition_end.to_string();
                                crate::maintenance_driver::execute_column_scoped_merge(
                                    backend,
                                    schema,
                                    &plan.model_file.db_name_owned(),
                                    &inc_plan.config.unique_key,
                                    &contribution,
                                    &bound,
                                    &inc_plan.timeseries.partition_column,
                                    &conv_ts,
                                    &compiled.sql,
                                    &compiled.output_columns,
                                    suppression,
                                    &partition,
                                    &retry_policy,
                                )
                                .await
                                .map_err(|e| anyhow::anyhow!("{}", e))?
                            }
                        }
                    } else if let Some(facts) = restricted_facts_this_batch {
                        // T3 (`docs/plans/20260715-composed-axes-
                        // conditional-maintenance.md` Phase E3): this
                        // model's creation cell is sourced (at least in
                        // part) by a maintained-model upstream edge whose
                        // recorded observed delta may license restricting
                        // this batch's recompute to the changed-key set
                        // (`resolve_recompute_restriction`'s two-factor
                        // admission: P1 skeleton-source closure `Closed` ∧ a
                        // non-empty delta — resolved inside
                        // `execute_delete_insert_with_delta_restriction`
                        // itself, reading the SAME `_smelt_observed_delta`
                        // table T5 writes). `restricted_facts_this_batch` is
                        // `None` (falling through to the ordinary branch
                        // below) whenever the target doesn't exist yet
                        // (the bootstrap `CREATE TABLE AS` case — restriction
                        // only ever applies to the ordinary incremental
                        // recompute over an already-materialized target,
                        // never the first-run materialization), the backend
                        // isn't DuckDB, or the resolved strategy isn't
                        // `DeleteInsert`. Falls back to the ordinary widened
                        // scan (byte-identical to the branch below) whenever
                        // the closure is `Open`/absent, the delta is absent
                        // or empty, or the model's row identity is not a
                        // single column — never a silent skip.
                        let region = smelt_logical::maintenance::emit::Region::for_axis(
                            partition.axis,
                            &partition.start,
                            &partition.end,
                        )
                        .map_err(anyhow::Error::msg)?;
                        let group =
                            crate::maintenance_driver::execute_delete_insert_with_delta_restriction(
                                backend,
                                schema,
                                &plan.model_file.db_name_owned(),
                                &partition.column,
                                &region,
                                &compiled.sql,
                                &compiled.body_sql,
                                facts.restrict_column.as_deref(),
                                facts.skeleton_source_closure.as_ref(),
                                crate::maintenance_driver::RestrictionDeltaSource::ModelEdge {
                                    upstream_model: &facts.upstream_model,
                                    window_start: &partition.start,
                                    window_end: &partition.end,
                                },
                                Some(&facts.region_write),
                                smelt_backend::maintenance_dialect(backend.dialect()),
                                &retry_policy,
                                &probe_policy_for_model(config, prior_runs, &plan.name),
                                &ledger_ensure_sqls,
                                &ledger_pre_write_sqls,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        let chunk = crate::reporter::ChunkInfo {
                            index: batch_idx,
                            total: inc_plan.batches.len(),
                            start: partition.start.clone(),
                            end: partition.end.clone(),
                        };
                        reporter.maintenance_statements(run_id, &plan.name, Some(&chunk), &group);
                        let row_count = backend
                            .get_row_count(schema, &plan.model_file.db_name_owned())
                            .await
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        smelt_backend::ExecutionResult {
                            model_name: plan.name.clone(),
                            duration: batch_start_time.elapsed(),
                            row_count,
                            preview: None,
                        }
                    } else if let Some(facts) = external_restricted_facts_this_batch {
                        // 27e: this model's `UpstreamMutation` cell is driven
                        // by an external `mutable_snapshot` source with no
                        // native change feed — the fingerprint sidecar's own
                        // diff (`RestrictionDeltaSource::ExternalSidecar`)
                        // may license restricting this batch's recompute to
                        // the synthesized changed-key set, resolved inside
                        // `execute_delete_insert_with_delta_restriction`
                        // itself. `external_restricted_facts_this_batch` is
                        // `None` (falling through to the ordinary branch
                        // below) under the same conditions as the model-edge
                        // route above.
                        let source_info = source_infos
                            .iter()
                            .find(|info| {
                                let segs = &info.address_segments;
                                let bare = match segs.split_first() {
                                    Some((first, rest)) if first == "sources" => rest.join("."),
                                    _ => segs.join("."),
                                };
                                bare == facts.source_name
                            })
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "model '{}' resolved a live external delta-restriction cell \
                                     on source '{}', but that source has no resolved physical \
                                     table",
                                    plan.name,
                                    facts.source_name
                                )
                            })?;
                        let source_table = source_info.db_name_for_target(model_target, schema);
                        let source_address = format!("smelt.sources.{}", facts.source_name);
                        let consumer_address =
                            format!("smelt.models.{}", plan.model_file.canonical_path());
                        let all_source_columns: Vec<String> = source_info
                            .columns
                            .iter()
                            .map(|c| c.name.clone())
                            .collect();
                        let source_key: Vec<String> = maint_source_facts
                            .iter()
                            .find(|s| s.name == facts.source_name)
                            .map(|s| s.unique_key.clone())
                            .unwrap_or_default();
                        let region = smelt_logical::maintenance::emit::Region::for_axis(
                            partition.axis,
                            &partition.start,
                            &partition.end,
                        )
                        .map_err(anyhow::Error::msg)?;
                        let group =
                            crate::maintenance_driver::execute_delete_insert_with_delta_restriction(
                                backend,
                                schema,
                                &plan.model_file.db_name_owned(),
                                &partition.column,
                                &region,
                                &compiled.sql,
                                &compiled.body_sql,
                                facts.restrict_column.as_deref(),
                                facts.skeleton_source_closure.as_ref(),
                                crate::maintenance_driver::RestrictionDeltaSource::ExternalSidecar {
                                    source_address: &source_address,
                                    source_table: &source_table,
                                    source_key: &source_key,
                                    projection: &facts.projection,
                                    all_source_columns: &all_source_columns,
                                    model_sql: &compiled.body_sql,
                                    consumer_address: &consumer_address,
                                },
                                Some(&facts.region_write),
                                smelt_backend::maintenance_dialect(backend.dialect()),
                                &retry_policy,
                                &probe_policy_for_model(config, prior_runs, &plan.name),
                                &ledger_ensure_sqls,
                                &ledger_pre_write_sqls,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        // Refresh the sidecar to the source's CURRENT
                        // content only after the write that consumed the
                        // diff has succeeded — riding the same ordering the
                        // repair family's own sidecar refresh follows
                        // (`RepairSidecarRefresh`'s doc comment), so a
                        // failed write never advances the sidecar past a
                        // change it did not actually consume.
                        crate::maintenance_driver::refresh_fingerprint_sidecar(
                            backend,
                            schema,
                            &source_address,
                            &source_table,
                            &source_key,
                            &facts.projection,
                            &all_source_columns,
                            &compiled.body_sql,
                            &consumer_address,
                            &group,
                        )
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?;
                        let chunk = crate::reporter::ChunkInfo {
                            index: batch_idx,
                            total: inc_plan.batches.len(),
                            start: partition.start.clone(),
                            end: partition.end.clone(),
                        };
                        reporter.maintenance_statements(run_id, &plan.name, Some(&chunk), &group);
                        let row_count = backend
                            .get_row_count(schema, &plan.model_file.db_name_owned())
                            .await
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        smelt_backend::ExecutionResult {
                            model_name: plan.name.clone(),
                            duration: batch_start_time.elapsed(),
                            row_count,
                            preview: None,
                        }
                    } else {
                        // Observability: report the region DELETE+INSERT
                        // group this batch is about to execute — the same
                        // emitter call `Backend::delete_and_insert_transactional`
                        // makes to build what it actually executes
                        // (`docs/specs/incremental_models.md` §"Statement
                        // emission (single owner)"). Pure function, same
                        // inputs, so the reported text cannot drift from the
                        // executed text.
                        //
                        // `schema.table` is the correct fully-qualified name
                        // for every dialect this runtime path is exercised
                        // against today (DuckDB); a catalog-qualifying
                        // backend (Spark) would need its own qualified name
                        // here, so the report is scoped to DuckDB until a
                        // generic `Backend::qualified_table_name` exists —
                        // Spark's *executed* text is still correct (its own
                        // `delete_and_insert_transactional` override builds
                        // it), only this runtime-side report is narrowed.
                        if backend.dialect() == smelt_backend::SqlDialect::DuckDB
                            && matches!(
                                resolved_strategy,
                                smelt_backend::IncrementalStrategy::DeleteInsert
                            )
                        {
                            let table_name =
                                format!("{schema}.{}", plan.model_file.db_name_owned());
                            let region = smelt_logical::maintenance::emit::Region::for_axis(
                                partition.axis,
                                &partition.start,
                                &partition.end,
                            )
                            .map_err(anyhow::Error::msg)?;
                            let group = smelt_logical::maintenance::emit::emit_delete_insert(
                                &table_name,
                                &partition.column,
                                &region,
                                &compiled.sql,
                                smelt_backend::maintenance_dialect(backend.dialect()),
                            );
                            let chunk = crate::reporter::ChunkInfo {
                                index: batch_idx,
                                total: inc_plan.batches.len(),
                                start: partition.start.clone(),
                                end: partition.end.clone(),
                            };
                            reporter.maintenance_statements(
                                run_id,
                                &plan.name,
                                Some(&chunk),
                                &group,
                            );
                        }

                        // Reconciliation ledger: this batch's own
                        // `ledger_ensure_sqls`/`ledger_pre_write_sqls`, hoisted above
                        // the T3/27e dispatch fork (see the comment there), record
                        // the SAME region-recompute reset this plain widened-scan
                        // branch used to build itself.
                        let strategy = MaterializationStrategy::Incremental {
                            partition,
                            strategy: resolved_strategy.clone(),
                            unique_key: inc_plan.config.unique_key.clone(),
                        };

                        let db_name = plan.model_file.db_name_owned();
                        retry_statement_group(request, run_id, &plan.name, reporter, || {
                            backend.execute_model_incremental_with_bookkeeping(
                                schema,
                                &db_name,
                                &compiled.sql,
                                Materialization::Table,
                                strategy.clone(),
                                false,
                                &ledger_ensure_sqls,
                                &ledger_pre_write_sqls,
                            )
                        })
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?
                    };

                    total_rows += exec_result.row_count;
                    total_rows_overall += exec_result.row_count;

                    let batch_duration = batch_start_time.elapsed();
                    reporter.batch_completed(
                        run_id,
                        &plan.name,
                        batch_idx,
                        inc_plan.batches.len(),
                        exec_result.row_count,
                        batch_duration,
                    );
                }

                // Recorded once for the whole run — the observed fingerprint
                // `mutation_gate` carries was itself computed once, before
                // the batch loop, and every batch that dispatched the merge
                // did so against that SAME observed state
                // (`docs/specs/incremental_models.md` §"When a mutation
                // cell dispatches").
                if used_column_scoped_merge {
                    if let Some((source, _cell, _suppression)) = column_scoped_cell.as_ref() {
                        record_upstream_mutation_baseline(
                            mutation_gate,
                            source,
                            file_store,
                            state_io_lock,
                        )
                        .await;
                    }
                }

                // Manifest entry for the model
                let (start_str, end_str) = match (start_date, end_date) {
                    (Some(s), Some(e)) => (
                        s.format("%Y-%m-%d").to_string(),
                        e.format("%Y-%m-%d").to_string(),
                    ),
                    _ => (String::new(), String::new()),
                };
                let strategy_label = if used_in_place_update {
                    "in_place_update".to_string()
                } else if used_column_scoped_merge {
                    "column_scoped_merge".to_string()
                } else {
                    format!("{:?}", resolved_strategy).to_lowercase()
                };

                // `contract.deferral`: prove ledger-proven work subsumption
                // on the run that catches a previously-deferred window up —
                // both legs are ledger facts, never inferred
                // (`docs/outcomes/20260809-contract-lattice-v1/outcome.md`
                // phase 5 decision log): a prior run manifest actually
                // recorded `skipped_deferral` for this model, AND this
                // run's own write range covers the pending window computed
                // from the SAME pre-run frontier snapshot the skip decision
                // used.
                let subsumed = match (start_date, end_date) {
                    (Some(s), Some(e)) => {
                        // The nearest prior manifest that actually recorded
                        // an entry for THIS model (`prior_runs` is
                        // newest-first) — a run that never selected this
                        // model at all must not count as "no prior skip"
                        // and mask an older recorded one.
                        let prior_recorded_skip = prior_runs
                            .iter()
                            .find_map(|m| m.models.get(&plan.name))
                            .is_some_and(|rec| rec.strategy == "skipped_deferral");
                        crate::contract_probes::subsumed_window(
                            deferral_pending.get(&plan.name).copied(),
                            prior_recorded_skip,
                            s,
                            e,
                        )
                    }
                    _ => None,
                };

                manifest_entries.insert(
                    plan.name.clone(),
                    ModelRunRecord {
                        strategy: strategy_label,
                        time_range: Some(TimeRangeRecord {
                            start: start_str.clone(),
                            end: end_str.clone(),
                        }),
                        partitions_updated: vec![],
                        row_count: total_rows,
                        duration_ms: model_start.elapsed().as_millis() as u64,
                        batch_safety: Some("incremental".to_string()),
                        outcome: smelt_state::RunOutcomeKind::Success,
                        definition_hash: compute_model_hash(&plan.sql),
                        error: None,
                        retry_count: sink.retry_count(),
                        probes: model_probe_records,
                        subsumed,
                        deferred_cells: Vec::new(),
                    },
                );

                // Update interval store. A column-scoped MERGE (MP11) still
                // writes exactly the `[start_str, end_str)` run window each
                // batch was already scoped to (`compiled.sql` carries the
                // same `inject_time_filter`/`inject_source_filters` clamp a
                // DELETE+INSERT batch would have used) — only the physical
                // write technique differs, so the interval store's record
                // stays accurate regardless of which technique wrote it.
                {
                    // Whole-store load-modify-save critical section
                    // (`state_io_lock`, declared before the wavefront
                    // scheduler) — `intervals.json` is one JSON blob
                    // covering every model, so two models finishing in the
                    // same wave must not race a save that silently drops
                    // the other's write.
                    let _io_guard = state_io_lock.lock().await;
                    if let Ok(mut interval_store) = file_store.load_intervals() {
                        let model_hash = compute_model_hash(&plan.sql);
                        let intervals = interval_store.get_or_create(&plan.name, &model_hash);
                        intervals.record_interval(&start_str, &end_str);
                        // `contract.cells[].deferral`: this fold just ran,
                        // so every one of this model's own declaring cells
                        // (resolved once in the pre-run pass, not
                        // re-derived here) advances its frontier to this
                        // run's own end — a run past the cell window (or an
                        // unlicensed `Proceed`) always folds the whole row,
                        // so every declaring cell is caught up regardless of
                        // which one(s) actually licensed the coverage check
                        // (`docs/outcomes/20260815-definition-delta-migrate/
                        // phases/14-plan.md`).
                        if let Some(addresses) = deferral_fold_addresses.get(&plan.name) {
                            crate::contract_probes::advance_cell_frontiers(
                                &mut interval_store,
                                &plan.name,
                                &model_hash,
                                addresses,
                                &end_str,
                            );
                        }
                        let _ = file_store.save_intervals(&interval_store);
                    }
                }

                // Per-source landed-delta recording (P10 v1: `docs/specs/sources.md`
                // §"World-facts admission consumes"): for every source this
                // model consumed (`maint_source_facts`, already resolved
                // above against `source_infos` with the same bare-name
                // convention `smelt-db::maintenance_plan` uses), record that
                // `[start_str, end_str)` — this run's own window, the v1
                // proxy for "what landed" — is now reflected on that
                // source's own partition axis. An append-only clocked
                // source (`partition_col: Some(_)`, `mutation: AppendOnly`)
                // is interval-diffed against prior coverage; a mutable
                // snapshot or unclocked source (`partition_col: None`) has
                // no interval representation and always resolves to
                // `LandedDelta::WholeTable` — never a silent no-op
                // (`incremental_models.md` §"Forward propagation").
                if !start_str.is_empty() && !end_str.is_empty() {
                    // Same whole-store critical section rationale as the
                    // interval store above — `landed_deltas.json`.
                    let _io_guard = state_io_lock.lock().await;
                    if let Ok(mut landed_deltas) = file_store.load_landed_deltas() {
                        for sf in &maint_source_facts {
                            let posture = if sf.partition_col.is_none() {
                                SourceMutationPosture::Unclocked
                            } else {
                                match sf.mutation {
                                    smelt_logical::maintenance::MutationProfile::AppendOnly => {
                                        SourceMutationPosture::AppendOnly
                                    }
                                    smelt_logical::maintenance::MutationProfile::MutableSnapshot
                                    // A change feed has no interval representation
                                    // either — its landed delta is recorded the same
                                    // whole-table way a mutable snapshot's is.
                                    | smelt_logical::maintenance::MutationProfile::ChangeFeed => {
                                        SourceMutationPosture::MutableSnapshot
                                    }
                                }
                            };
                            record_landing(
                                &mut landed_deltas,
                                &sf.name,
                                posture,
                                &start_str,
                                &end_str,
                            );
                        }
                        let _ = file_store.save_landed_deltas(&landed_deltas);
                    }
                }

                // Reconciliation ledger: engine-resident now
                // (`docs/specs/incremental_models.md` §"The reconciliation
                // ledger"; `docs/outcomes/20260904-state-residency/
                // outcome.md`). Each DuckDB DeleteInsert batch's own region
                // recompute records its recompute-reset in `_smelt_ledger`,
                // in the SAME transaction as its write — see the
                // `ledger_reset_sqls`/`execute_model_incremental_with_bookkeeping`
                // call inside the batch loop above. `.smelt/reconciliation.json`
                // no longer exists.
                } // 'run_dispatch_or_batches

                Ok(())
            }
            None => {
                // Full refresh
                let clean_sql = smelt_parser::strip_frontmatter(&plan.sql);
                let compiler = compilers.get(model_target);
                let resolver = &ephemeral_resolvers[model_target];
                let compiled = compiler.compile_with_sql_and_ephemerals(
                    &plan.model_file,
                    schema,
                    &clean_sql,
                    resolver,
                )?;
                reporter.model_compiled(run_id, &plan.name, &compiled.sql);

                // Accumulates every probe's held/skipped outcome for this
                // full-refresh write, for `ModelRunRecord.probes`
                // (`docs/specs/run_state.md` §"Run manifest").
                let mut model_probe_records: Vec<smelt_state::ProbeRecord> = Vec::new();

                // Live dispatch of the declared model-scoped probes
                // (`timeseries.assert_monotonic`, `functional_dependencies:`,
                // `bounded_domain:`) before the materialization write —
                // a firing probe fails the run before anything is written
                // (`docs/specs/model_properties.md` §"Probe obligation").
                let declared_probes = crate::model_probes::declared_model_probes(
                    &plan.name,
                    &format!("{}.{} full refresh", schema, plan.model_file.db_name_owned()),
                    plan.model_file.metadata.as_deref(),
                    plan.model_file
                        .metadata
                        .as_ref()
                        .and_then(|m| m.timeseries.as_ref()),
                    &compiled.sql,
                    smelt_backend::maintenance_dialect(backend.dialect()),
                );
                model_probe_records.extend(
                    crate::model_probes::dispatch_declared_model_probes(
                        backend,
                        &probe_policy_for_model(config, prior_runs, &plan.name),
                        &declared_probes,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?,
                );

                // Live dispatch of the source append-only posture probe
                // (`docs/specs/model_properties.md` §"Probe obligation", row
                // `mutation_profile.kind: append_only`) before the
                // full-refresh write — same pre-write obligation as the
                // incremental-batch site above.
                {
                    let _io_guard = state_io_lock.lock().await;
                    let source_postures = file_store
                        .load_source_postures()
                        .map_err(|e| anyhow::anyhow!("{}", e))?;
                    let source_probes = crate::source_probes::append_only_posture_probes(
                        &plan.name,
                        &format!("{}.{} full refresh", schema, plan.model_file.db_name_owned()),
                        &plan.model_file,
                        source_infos,
                        &source_postures,
                        model_target,
                        schema,
                        smelt_backend::maintenance_dialect(backend.dialect()),
                    );
                    if !source_probes.is_empty() {
                        let (refreshed, records) =
                            crate::source_probes::dispatch_and_record_append_only_postures(
                                backend,
                                &probe_policy_for_model(config, prior_runs, &plan.name),
                                &source_probes,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        model_probe_records.extend(records);
                        if !refreshed.is_empty() {
                            let mut source_postures = source_postures;
                            for r in refreshed {
                                source_postures.record(&r.source_address, r.partitions);
                            }
                            let _ = file_store.save_source_postures(&source_postures);
                        }
                    }
                }

                let mat = match plan.materialization {
                    smelt_core::config::Materialization::Table => Materialization::Table,
                    smelt_core::config::Materialization::View => Materialization::View,
                    smelt_core::config::Materialization::Ephemeral => {
                        unreachable!("Ephemeral models should be inlined as CTEs, not executed")
                    }
                };

                // A self-referential model has no well-defined single-shot
                // full refresh: `CREATE TABLE … AS SELECT …` cannot resolve
                // the self-reference, and `execute_model`'s ordinary full-
                // refresh contract (drop, then CTAS) would in any case drop
                // the very prior state the SELECT reads before the SELECT
                // ever runs. This arm is the unwindowed sibling of the
                // incremental path's own first-run bootstrap
                // (`docs/specs/incremental_shapes.md` §"First-run and
                // backfill" — "First-run bootstrap for a self-referential
                // model"): drop, bootstrap an EMPTY target from the
                // resolved output schema, then `INSERT` the compiled
                // query's rows — the self-read sees no prior state, the
                // same starting condition an incremental run's first
                // partition gets. There is no window to sequence multiple
                // partitions over here (this arm only runs when `smelt
                // build`/`smelt run` supplied no `--event-time-start`/
                // `--event-time-end`), so the whole unfiltered query
                // executes as one INSERT — correct for the model's FIRST
                // build, the same one-shot shape a plain (non-self-
                // referential) full refresh already is.
                let exec_result = if plan.refresh
                    == smelt_core::config::RefreshStrategy::MaterializedView
                {
                    // `refresh: materialized_view` (`docs/specs/materialized_view.md`):
                    // delegate to the backend's native incremental-view
                    // maintenance instead of the ordinary drop+CTAS/CREATE
                    // VIEW path `Backend::execute_model` runs. The
                    // `supports_native_ivm` gate (`compile::check_native_ivm_gate`)
                    // already refused compilation above for any backend
                    // without native IVM, so this arm only runs against a
                    // backend that overrides `create_materialized_view_as`
                    // — the default's `UnsupportedFeature` error is a
                    // second line of defense, not the primary enforcement.
                    let db_name = plan.model_file.db_name_owned();
                    retry_statement_group(request, run_id, &plan.name, reporter, || {
                        backend.create_materialized_view_as(schema, &db_name, &compiled.sql)
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;

                    let row_count = backend
                        .get_row_count(schema, &plan.model_file.db_name_owned())
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?;
                    smelt_backend::ExecutionResult {
                        model_name: plan.model_file.db_name_owned(),
                        duration: StdDuration::from_millis(0),
                        row_count,
                        preview: None,
                    }
                } else if crate::compile::is_self_referential(&plan.model_file)
                    && matches!(mat, Materialization::Table)
                {
                    backend
                        .drop_view_if_exists(schema, &plan.model_file.db_name_owned())
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?;
                    backend
                        .drop_table_if_exists(schema, &plan.model_file.db_name_owned())
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?;

                    bootstrap_self_ref_empty_target(
                        request,
                        backend,
                        schema,
                        &plan.model_file,
                        &plan.name,
                        upstream_schemas_for_bootstrap,
                        reporter,
                        run_id,
                    )
                    .await?;

                    let db_name = plan.model_file.db_name_owned();
                    retry_statement_group(request, run_id, &plan.name, reporter, || {
                        backend.insert_into_from_query(schema, &db_name, &compiled.sql)
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;

                    let row_count = backend
                        .get_row_count(schema, &plan.model_file.db_name_owned())
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?;
                    smelt_backend::ExecutionResult {
                        model_name: plan.model_file.db_name_owned(),
                        duration: StdDuration::from_millis(0),
                        row_count,
                        preview: None,
                    }
                } else {
                    let db_name = plan.model_file.db_name_owned();
                    retry_statement_group(request, run_id, &plan.name, reporter, || {
                        backend.execute_model(schema, &db_name, &compiled.sql, mat, false)
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?
                };

                total_rows = exec_result.row_count;
                total_rows_overall += exec_result.row_count;

                // Save deployed schema for full-refresh models (best-effort).
                // This enables `smelt diff` to report schema changes on subsequent runs.
                {
                    let db_guard = db.lock().await;
                    let inferred_columns = crate::schema_evolution::infer_deployed_columns(
                        &db_guard,
                        &plan.model_file,
                    );
                    if !inferred_columns.is_empty() {
                        let db_table_name = plan.model_file.db_name_owned();
                        let existing_version = file_store
                            .load_schema(&db_table_name)
                            .ok()
                            .flatten()
                            .map(|s| s.version);
                        if let Err(e) = crate::schema_evolution::save_deployed_schema(
                            file_store,
                            &db_table_name,
                            &plan.sql,
                            &inferred_columns,
                            existing_version,
                            // Read from the model's own declared metadata,
                            // not `plan.incremental` — a first-ever run of
                            // a `refresh: incremental` model executes via
                            // this full-refresh save path (no existing
                            // table to incrementally build against yet),
                            // where `plan.incremental` is `None` even
                            // though the model does declare `timeseries:`.
                            plan.model_file
                                .metadata
                                .as_ref()
                                .and_then(|m| m.timeseries.as_ref())
                                .map(|ts| ts.partition_column.as_str()),
                        ) {
                            tracing::warn!(
                                "Failed to save deployed schema for '{}': {}",
                                plan.name,
                                e
                            );
                        }
                    }
                }

                manifest_entries.insert(
                    plan.name.clone(),
                    ModelRunRecord {
                        strategy: if plan.refresh
                            == smelt_core::config::RefreshStrategy::MaterializedView
                        {
                            "materialized_view".to_string()
                        } else {
                            "full_refresh".to_string()
                        },
                        time_range: None,
                        partitions_updated: vec![],
                        row_count: exec_result.row_count,
                        duration_ms: model_start.elapsed().as_millis() as u64,
                        batch_safety: None,
                        outcome: smelt_state::RunOutcomeKind::Success,
                        definition_hash: compute_model_hash(&plan.sql),
                        error: None,
                        retry_count: sink.retry_count(),
                        probes: model_probe_records,
                        subsumed: None,
                        deferred_cells: Vec::new(),
                    },
                );

                Ok(())
            }
        };

        result?;

        // ── First-deployment schema baseline (incremental models) ────────
        // The schema-evolution gate above can only diff against a stored
        // deployed schema. Full-refresh models save theirs inside their own
        // execution branch, but the incremental branch never did — leaving
        // the gate permanently on `FirstDeployment`: `smelt diff` reported
        // the model as new forever, and an added column crashed the next
        // incremental INSERT instead of being ALTERed in. Save a baseline
        // (best-effort, like the full-refresh save) the first time an
        // incremental model executes successfully; `check_and_migrate`
        // takes over versioning from then on.
        if plan.incremental.is_some() {
            let db_table_name = plan.model_file.db_name_owned();
            let already_stored = file_store
                .load_schema(&db_table_name)
                .ok()
                .flatten()
                .is_some();
            if !already_stored {
                let inferred_columns = {
                    let db_guard = db.lock().await;
                    crate::schema_evolution::infer_deployed_columns(&db_guard, &plan.model_file)
                };
                if !inferred_columns.is_empty() {
                    if let Err(e) = crate::schema_evolution::save_deployed_schema(
                        file_store,
                        &db_table_name,
                        &plan.sql,
                        &inferred_columns,
                        None,
                        plan.incremental
                            .as_ref()
                            .map(|inc| inc.timeseries.partition_column.as_str()),
                    ) {
                        tracing::warn!("Failed to save deployed schema for '{}': {}", plan.name, e);
                    }
                }
            }
        }

        let model_duration = model_start.elapsed();
        reporter.model_completed(run_id, &plan.name, total_rows, model_duration);
        // ── Check seam B: incremental / full-refresh arm ─────────────────────
        if request.run_checks {
            let (outcomes, to_skip) = run_model_checks(
                &plan.name,
                checks_by_model,
                compilers,
                backends,
                target_assignments,
                ephemeral_resolvers,
                config.as_ref(),
                upstream_map,
                selected,
                reporter,
                run_id,
            )
            .await;
            check_results.extend(outcomes);
            skip_set.extend(to_skip);
        }

        Ok(ModelOutcome::Completed(ModelSuccess {
            manifest_entries,
            check_results,
            skip_set,
            rows: total_rows,
        }))
        }
        .await;
            (sink, outcome)
        }
    };

    // ── Wavefront scheduler ─────────────────────────────────────────────
    // `jobs` bounds how many models may be IN FLIGHT concurrently; `waves`
    // (topological layers over the selected set,
    // `DependencyGraph::execution_waves`) bounds WHICH models may start
    // concurrently — a model never starts until every selected upstream
    // dependency's wave has fully drained. Reporter events and manifest
    // entries are buffered per model (`EventSink`) and flushed strictly in
    // `execution_order` (`model_plans`) index sequence via `next_flush`, so
    // the observable output is identical regardless of `jobs` or actual
    // completion order (`docs/plans/20260719-prod-w2-operability.md` Phase
    // 5).
    let jobs: usize = request
        .jobs
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        })
        .max(1);

    let selected_names: HashSet<String> = model_plans.iter().map(|p| p.name.clone()).collect();
    let waves: Vec<Vec<String>> = {
        let graph_lock_waves = graph.lock().await;
        graph_lock_waves.execution_waves(&selected_names)?
    };
    let index_of: HashMap<&str, usize> = model_plans
        .iter()
        .enumerate()
        .map(|(i, p)| (p.name.as_str(), i))
        .collect();

    let semaphore = tokio::sync::Semaphore::new(jobs);
    let mut pending: HashMap<usize, (EventSink, Result<ModelOutcome>)> = HashMap::new();
    let mut next_flush: usize = 0;
    let mut cancelled = false;
    // Every model whose execution raised an error, in flush order — not
    // just the first. A wave dispatches its models concurrently, so more
    // than one can fail before the scheduler notices; recording all of them
    // (rather than first-wins) is what lets the abort path below give every
    // failing model its own `Failed` manifest entry with its own error text
    // instead of silently downgrading the rest to `skipped`
    // (`docs/plans/20260719-prod-w2-operability.md` Phase 8 unblock
    // decision: "let the in-flight wave finish, record every failure, then
    // abort").
    let mut aborted: Vec<(String, anyhow::Error)> = Vec::new();

    macro_rules! flush_ready {
        () => {
            while let Some((sink, result)) = pending.remove(&next_flush) {
                let model_name = &model_plans[next_flush].name;
                sink.replay(reporter, &run_id, model_name);
                match result {
                    Ok(ModelOutcome::Completed(success)) => {
                        manifest.models.extend(success.manifest_entries);
                        check_results.extend(success.check_results);
                        skip_set.extend(success.skip_set);
                        total_rows_overall += success.rows;
                    }
                    Ok(ModelOutcome::Cancelled) => {
                        cancelled = true;
                    }
                    Err(e) => {
                        aborted.push((model_name.clone(), e));
                    }
                }
                next_flush += 1;
            }
        };
    }

    'waves: for wave in &waves {
        if cancelled || !aborted.is_empty() {
            break 'waves;
        }
        let mut in_flight = futures::stream::FuturesUnordered::new();
        for name in wave {
            let idx = index_of[name.as_str()];
            let already_skip = request.run_checks && skip_set.contains(name);
            let sem = &semaphore;
            let task = execute_one_model(idx, already_skip);
            in_flight.push(async move {
                let _permit = sem.acquire().await.expect("semaphore is never closed");
                let (sink, outcome) = task.await;
                (idx, sink, outcome)
            });
        }
        while let Some((idx, sink, outcome)) = in_flight.next().await {
            pending.insert(idx, (sink, outcome));
        }
        flush_ready!();
    }
    // Final catch-up flush: an aborted/cancelled run may leave a later
    // wave's already-completed results sitting behind an index whose model
    // never started (its wave was never scheduled) — still record every
    // outcome that DID complete
    // (`docs/plans/20260719-prod-w2-operability.md` Phase 5: "in-flight
    // models drain; all outcomes recorded").
    {
        let mut remaining: Vec<usize> = pending.keys().copied().collect();
        remaining.sort_unstable();
        for idx in remaining {
            if let Some((sink, result)) = pending.remove(&idx) {
                let model_name = &model_plans[idx].name;
                sink.replay(reporter, run_id, model_name);
                match result {
                    Ok(ModelOutcome::Completed(success)) => {
                        manifest.models.extend(success.manifest_entries);
                        check_results.extend(success.check_results);
                        skip_set.extend(success.skip_set);
                        total_rows_overall += success.rows;
                    }
                    Ok(ModelOutcome::Cancelled) => {
                        cancelled = true;
                    }
                    Err(e) => {
                        aborted.push((model_name.clone(), e));
                    }
                }
            }
        }
    }

    if cancelled {
        // Persist the manifest even on cancellation — a cancelled run (e.g.
        // via `smelt-ui`'s stop button, `CancellationToken`) must leave a
        // resumable manifest exactly like a hard-error abort does, since
        // `--resume` depends on reading it back
        // (`docs/specs/run_state.md` §"Run manifest", §"`--resume`
        // semantics"). Every selected model that never got a manifest entry
        // (its wave never started, or it was mid-flight when the
        // cancellation happened) is recorded `skipped` — never a silent
        // omission.
        for plan in model_plans.iter() {
            manifest
                .models
                .entry(plan.name.clone())
                .or_insert_with(|| ModelRunRecord {
                    strategy: "skipped".to_string(),
                    time_range: None,
                    partitions_updated: vec![],
                    row_count: 0,
                    duration_ms: 0,
                    batch_safety: Some("skipped".to_string()),
                    outcome: smelt_state::RunOutcomeKind::Skipped,
                    definition_hash: compute_model_hash(&plan.sql),
                    error: None,
                    retry_count: 0,
                    probes: Vec::new(),
                    subsumed: None,
                    deferred_cells: Vec::new(),
                });
        }
        // `completed_at` stays `None` — an incomplete run, exactly what
        // `--resume` looks for.
        if let Err(e) = file_store.save_run(&manifest) {
            tracing::warn!("Failed to save run manifest for cancelled run: {}", e);
        }
        if let Err(e) = write_run_report(file_store, &manifest) {
            tracing::warn!("Failed to write run report for cancelled run: {}", e);
        }
        reporter.run_cancelled(run_id);
        return Ok(build_outcome(
            run_id,
            run_start,
            None,
            manifest,
            total_rows_overall,
            vec![],
        ));
    }
    if !aborted.is_empty() {
        // Persist the manifest even on failure — a run that aborts partway
        // through must not silently drop the outcomes it DID observe,
        // since `--resume` depends on reading them back
        // (`docs/specs/run_state.md` §"Run manifest", §"`--resume`
        // semantics"). Every model whose execution raised an error this run
        // gets its own `failed` entry with its own error text (never just
        // the first — the rest downgraded to `skipped` would silently
        // discard real failures); every other selected model that never got
        // a manifest entry (its wave never started, or it was mid-flight
        // when the abort happened) is `skipped` — every model smelt
        // considered this run gets an entry, never a silent omission.
        for (model, error) in &aborted {
            manifest.models.entry(model.clone()).or_insert_with(|| {
                let definition_hash = model_plans
                    .iter()
                    .find(|p| &p.name == model)
                    .map(|p| compute_model_hash(&p.sql))
                    .unwrap_or_default();
                ModelRunRecord {
                    strategy: "failed".to_string(),
                    time_range: None,
                    partitions_updated: vec![],
                    row_count: 0,
                    duration_ms: 0,
                    batch_safety: None,
                    outcome: smelt_state::RunOutcomeKind::Failed,
                    definition_hash,
                    error: Some(error.to_string()),
                    retry_count: 0,
                    probes: Vec::new(),
                    subsumed: None,
                    deferred_cells: Vec::new(),
                }
            });
        }
        for plan in model_plans.iter() {
            manifest
                .models
                .entry(plan.name.clone())
                .or_insert_with(|| ModelRunRecord {
                    strategy: "skipped".to_string(),
                    time_range: None,
                    partitions_updated: vec![],
                    row_count: 0,
                    duration_ms: 0,
                    batch_safety: Some("skipped".to_string()),
                    outcome: smelt_state::RunOutcomeKind::Skipped,
                    definition_hash: compute_model_hash(&plan.sql),
                    error: None,
                    retry_count: 0,
                    probes: Vec::new(),
                    subsumed: None,
                    deferred_cells: Vec::new(),
                });
        }
        // `completed_at` stays `None` — an incomplete run, exactly what
        // `--resume` looks for.
        if let Err(e) = file_store.save_run(&manifest) {
            tracing::warn!("Failed to save run manifest for failed run: {}", e);
        }
        for (model, error) in &aborted {
            reporter.run_failed(run_id, Some(model), &error.to_string());
        }
        if let Err(e) = write_run_report(file_store, &manifest) {
            tracing::warn!("Failed to write run report for failed run: {}", e);
        }
        let (_first_model, first_error) = aborted.into_iter().next().expect("checked non-empty");
        return Err(first_error);
    }

    manifest.completed_at = Some(Utc::now());
    if let Err(e) = file_store.save_run(&manifest) {
        tracing::warn!("Failed to save run manifest: {}", e);
    }
    if let Err(e) = write_run_report(file_store, &manifest) {
        tracing::warn!("Failed to write run report: {}", e);
    }

    // Stale schema cleanup: remove .smelt/schemas/<name>.json entries for models
    // that no longer exist in the project. Uses db-names (not canonical paths)
    // so sub-directory models match correctly.
    {
        let current_names: std::collections::HashSet<String> =
            all_models.iter().map(|m| m.db_name_owned()).collect();
        for orphan in file_store
            .list_deployed_model_names()
            .into_iter()
            .filter(|n| !current_names.contains(n))
        {
            if let Err(e) = file_store.delete_schema(&orphan) {
                tracing::warn!("Failed to delete stale schema for '{}': {}", orphan, e);
            } else {
                tracing::debug!("Removed stale schema for deleted model '{}'", orphan);
            }
        }
    }

    let total_duration: StdDuration = execution_start.elapsed();
    reporter.run_completed(run_id, total_rows_overall, total_duration);

    Ok(build_outcome(
        run_id,
        run_start,
        Some(Utc::now()),
        manifest,
        total_rows_overall,
        check_results,
    ))
}
