//! `execute_project` — the shared run pipeline.
//!
//! Composes the analysis layer (`smelt-db`, `smelt-core`, `smelt-planner`),
//! the compile layer (`smelt_runtime::compile`), and the selection layer
//! (`smelt_runtime::select`) into the full per-model execute loop. Both
//! `smelt-cli`'s `commands/run.rs` and `smelt-ui`'s `run_manager.rs`
//! consume this function via a `RunReporter` adapter.
//!
//! This file owns the model-plan construction (batch dispatch per
//! `BatchSafety` shape), the per-model compile+execute loop (full refresh,
//! incremental batches, and `cumulative_aggregate` dispatch via
//! `crate::cumulative`), cancellation handling, manifest writes, and
//! interval-store updates.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use tokio_util::sync::CancellationToken;

use smelt_backend::{Backend, Materialization, MaterializationStrategy, PartitionRange};
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_planner::Frontmatter;
use smelt_state::file_store::FileStore;
use smelt_state::intervals::compute_model_hash;
use smelt_state::{ModelRunRecord, RunManifest, TimeRangeRecord};

use crate::compile::CompilerRegistry;
use crate::reporter::RunReporter;
use crate::safety::{build_model_graph, check_bound_derivation, check_planner_safety};
use crate::schema_evolution::{
    check_and_migrate, ddl_backend_for_dialect, extract_evolution_maps, infer_deployed_columns,
};
use crate::select::{select_executable_models, SelectionRequest};
use crate::transformer::{inject_time_filter, TimeRange};
use crate::types::{ExecuteRequest, RunOutcome};
use crate::windowing::{compute_incremental_windows, IncrementalBatch};
use crate::{build_fn_body_map, expand_function_calls, EphemeralResolver, UpstreamSchemas};

/// Plan for one model's execution. Internal to `execute_project` — the
/// public API is `ExecuteRequest` in / `RunOutcome` out.
struct ModelPlan {
    name: String,
    sql: String,
    materialization: smelt_core::config::Materialization,
    incremental: Option<IncrementalPlan>,
    model_file: smelt_core::ModelFile,
}

struct IncrementalPlan {
    config: smelt_core::IncrementalConfig,
    timeseries: smelt_core::config::TimeseriesConfig,
    /// Batches with separate partition and filter ranges (bound-aware windowing).
    batches: Vec<IncrementalBatch>,
}

/// Future returned by `BackendFactory::create`. Pinned + boxed so the trait
/// stays object-safe; a `type` alias keeps the trait signature readable.
pub type BackendFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Box<dyn Backend>>> + Send + 'a>>;

/// Backend factory injected by the consumer. The UI and CLI know how to
/// build their backends (DuckDB, Spark, etc.) and may differ in cred
/// resolution / feature gating; the runtime stays agnostic.
pub trait BackendFactory: Send + Sync {
    fn create<'a>(
        &'a self,
        target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        project_dir: &'a Path,
    ) -> BackendFuture<'a>;
}

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
    if !cross_edges.is_empty() {
        tracing::info!(
            "Cross-engine references detected ({} transfer(s) via Parquet)",
            cross_edges.len()
        );
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

    // ── Time range parsing ──────────────────────────────────────────────
    let (start_date, end_date) = match (request.start.as_deref(), request.end.as_deref()) {
        (Some(s), Some(e)) => {
            let sd = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .with_context(|| format!("Invalid start date: {s}"))?;
            let ed = NaiveDate::parse_from_str(e, "%Y-%m-%d")
                .with_context(|| format!("Invalid end date: {e}"))?;
            if sd >= ed {
                anyhow::bail!("Start date must be before end date");
            }
            (Some(sd), Some(ed))
        }
        (None, None) => (None, None),
        _ => anyhow::bail!("Both start and end must be provided together (or neither)"),
    };

    // Function bodies, built up-front so batch-safety classification below can
    // see a lookback declared inside a `smelt.define` body (parity with the CLI
    // run path). Reused for the compile context further down.
    let fn_bodies = {
        let db_guard = db.lock().await;
        let db_ref: &smelt_db::Database = &db_guard;
        let workspace = smelt_db::Workspace::try_get(db_ref)
            .ok_or_else(|| anyhow::anyhow!("workspace not initialised in DB"))?;
        build_fn_body_map(db_ref, workspace)
    };

    // ── Diagnostic-parity gate (analysis ↔ build) ───────────────────────
    // Refuse to compile/execute if the analyzer (the same surface the LSP
    // publishes) reports any `Error`-severity diagnostic on a selected model
    // or in-DAG dep. Shared with the CLI run path via `crate::gate`. Spec:
    // `docs/specs/architecture.md` §"Diagnostic parity rule (analysis ↔ build)".
    //
    // Function-definition files are gated alongside the selected models:
    // function-body diagnostics (e.g. `CteCycle`) are reported against the
    // `smelt.define` file, not its callers, so omitting them would let a
    // selected model that calls a broken function build through this path while
    // the editor flags it red (a parity gap). Paths the DB has not registered
    // are skipped by the helper, so listing all function files is safe.
    {
        let mut model_paths: Vec<std::path::PathBuf> = selected
            .iter()
            .filter_map(|name| graph_lock.get_model(name).ok().map(|m| m.path.clone()))
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

    // ── Planner safety check + temporal bound derivation ───────────────
    // Guard: refuse to execute if any selected incremental model fails the
    // planner's incremental safety classifier or has an undefinable temporal
    // bound (a bare LAG/LEAD without RANGE BETWEEN INTERVAL). Both checks
    // mirror what the CLI previously did inline in `commands/run.rs` before
    // building the physical graph.
    //
    // `enforce_safety = false` (mirrors `--allow-downgrade`) demotes
    // refusals to `warn!` and lets the model fall back to full-table refresh.
    {
        let model_graph = build_model_graph(&selected, &graph_lock, &config);
        check_planner_safety(&model_graph, request.enforce_safety)?;
        check_bound_derivation(&model_graph, request.enforce_safety)?;
    }

    // ── Model-plan construction + ephemeral collection ──────────────────
    let mut model_plans: Vec<ModelPlan> = Vec::new();
    let mut total_batches: usize = 0;

    for model_name in &selected {
        let model = graph_lock.get_model(model_name)?;
        let metadata = model.metadata.as_deref();
        let frontmatter = Frontmatter::parse(&model.content);

        let inc_config = config
            .get_incremental_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| frontmatter.as_ref().and_then(|f| f.incremental.clone()));

        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));

        match (inc_config, ts_config, start_date, end_date) {
            (Some(inc), Some(ts), Some(start_date), Some(end_date)) => {
                // Resolve data latency from model column metadata for the event-time column.
                let data_latency_days = metadata
                    .and_then(|m| m.columns.get(&ts.event_time_column))
                    .and_then(|c| c.data_latency.as_ref())
                    .map(|l| l.to_days())
                    .unwrap_or(0);

                let full_range = TimeRange {
                    start: start_date.format("%Y-%m-%d").to_string(),
                    end: end_date.format("%Y-%m-%d").to_string(),
                };

                // Use bound-aware windowing: SQL temporal dependencies + data latency
                // determine filter widening (not just analyze_batch_safety context_days).
                let expanded_sql = expand_function_calls(&model.content, &fn_bodies);
                let inc_windows = compute_incremental_windows(
                    &ts,
                    &inc,
                    &expanded_sql,
                    data_latency_days,
                    &full_range,
                    request.batch_size_days,
                    request.per_partition,
                );

                let batches = inc_windows.batches;
                total_batches += batches.len();
                model_plans.push(ModelPlan {
                    name: model_name.clone(),
                    sql: model.content.clone(),
                    materialization: config.get_materialization_with_metadata(model_name, metadata),
                    incremental: Some(IncrementalPlan {
                        config: inc,
                        timeseries: ts,
                        batches,
                    }),
                    model_file: model.clone(),
                });
            }
            (Some(_inc), Some(_ts), _, _) => {
                // Incremental config present but no time window. Fall back to
                // full refresh; the model still compiles and executes.
                model_plans.push(ModelPlan {
                    name: model_name.clone(),
                    sql: model.content.clone(),
                    materialization: config.get_materialization_with_metadata(model_name, metadata),
                    incremental: None,
                    model_file: model.clone(),
                });
            }
            (Some(_inc), None, _, _) => {
                eprintln!(
                    "warning: model '{model_name}' has incremental: but no timeseries: — skipping incremental execution"
                );
                model_plans.push(ModelPlan {
                    name: model_name.clone(),
                    sql: model.content.clone(),
                    materialization: config.get_materialization_with_metadata(model_name, metadata),
                    incremental: None,
                    model_file: model.clone(),
                });
            }
            (None, _, _, _) => {
                model_plans.push(ModelPlan {
                    name: model_name.clone(),
                    sql: model.content.clone(),
                    materialization: config.get_materialization_with_metadata(model_name, metadata),
                    incremental: None,
                    model_file: model.clone(),
                });
            }
        }
    }

    let all_models: Vec<smelt_core::ModelFile> =
        graph_lock.iter_models().map(|(_, m)| m.clone()).collect();
    let mut ephemeral_models_by_target: HashMap<String, Vec<(String, String)>> = HashMap::new();
    // Project-wide `smelt.<path> → timeseries` map. Cumulative dispatch
    // looks up its driving source's ts here; the classifier resolves the
    // smelt.<path> key against this map (per cumulative_aggregate.md).
    let mut source_timeseries: smelt_planner::SourceTimeseriesMap = HashMap::new();
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
            if let Some(ts) = metadata.and_then(|m| m.timeseries.clone()) {
                let smelt_key = format!("smelt.{}", model.address_segments.join("."));
                source_timeseries.insert(smelt_key, ts);
            }
        }
    }
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
    compilers.set_upstream_schemas_all(upstream_schemas);
    if !fn_bodies.is_empty() {
        compilers.set_function_bodies_all(fn_bodies);
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
            compiler.build_ephemeral_resolver(models, schema),
        );
    }

    // ── Execute loop ────────────────────────────────────────────────────
    reporter.run_started(
        &run_id,
        &model_plans
            .iter()
            .map(|m| m.name.clone())
            .collect::<Vec<_>>(),
        total_batches,
    );

    let file_store = FileStore::new(project_dir);
    let mut manifest = RunManifest {
        run_id: run_id.clone(),
        started_at: run_start,
        completed_at: None,
        models: HashMap::new(),
    };

    let mut total_rows_overall: usize = 0;

    for (model_idx, plan) in model_plans.iter().enumerate() {
        if cancel.is_cancelled() {
            reporter.run_cancelled(&run_id);
            return Ok(build_outcome(&run_id, run_start, None, manifest, 0));
        }

        reporter.model_started(&run_id, &plan.name, model_idx, model_plans.len());

        let model_start = Instant::now();
        let mut total_rows = 0usize;

        let model_target = &target_assignments[&plan.name];
        let backend = backends[model_target].as_ref();
        let schema = &config.targets[model_target].schema;

        // ── Schema evolution gate (incremental models only) ──────────────
        // For incremental models that have a deployed schema, check whether
        // the inferred columns have changed and apply (or block) the required
        // migration. `force_full_refresh` overrides the planned incremental
        // strategy to a full-table rebuild when evolution requires it.
        let mut force_full_refresh = false;
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
                        let (column_defaults, backfill_exprs) =
                            extract_evolution_maps(plan.model_file.metadata.as_deref());
                        let target_config = config
                            .targets
                            .get(model_target)
                            .expect("target config must exist");
                        let table_format = plan
                            .model_file
                            .metadata
                            .as_deref()
                            .and_then(|m| m.format.as_ref())
                            .cloned()
                            .or_else(|| target_config.table_format());
                        let ddl_backend =
                            ddl_backend_for_dialect(backend.dialect(), table_format, None);
                        match check_and_migrate(
                            backend,
                            &file_store,
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
                        )
                        .await
                        {
                            Ok(result) => {
                                match crate::safety::should_force_full_refresh(
                                    &result,
                                    &plan.name,
                                    request.allow_column_removal,
                                    request.allow_full_refresh,
                                ) {
                                    Ok(should_refresh) => force_full_refresh = should_refresh,
                                    Err(e) => {
                                        reporter.run_failed(
                                            &run_id,
                                            Some(&plan.name),
                                            &e.to_string(),
                                        );
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
            }
        }

        // Cumulative-aggregate dispatch — handled separately from the
        // incremental / full-refresh branches because it has its own per-
        // partition merge loop (see `smelt_runtime::cumulative` and
        // `docs/specs/cumulative_aggregate.md`).
        if plan.materialization == smelt_core::config::Materialization::CumulativeAggregate {
            let db_table_name = plan.model_file.db_name_owned();
            let compiler = compilers.get(model_target);
            let resolver = &ephemeral_resolvers[model_target];

            let exec_result = match (start_date, end_date) {
                (Some(s), Some(e)) => {
                    let time_range = TimeRange {
                        start: s.format("%Y-%m-%d").to_string(),
                        end: e.format("%Y-%m-%d").to_string(),
                    };
                    crate::cumulative::execute_cumulative_aggregate(
                        backend,
                        &plan.model_file,
                        &compilers,
                        resolver,
                        model_target,
                        schema,
                        &db_table_name,
                        &time_range,
                        &source_timeseries,
                        false,
                    )
                    .await
                }
                _ => {
                    // No run window: single-shot full refresh of the
                    // cumulative SELECT. Matches CLI's behaviour for
                    // `smelt build` / `smelt run` without an event-time
                    // window.
                    let clean_sql = smelt_parser::strip_frontmatter(&plan.sql);
                    // Classify even on the no-window full-refresh path: a
                    // classifier rejection must REFUSE the model
                    // (cumulative_aggregate.md Constraint #10 — "No silent
                    // downgrade. … No fallback to full-refresh"). Without this,
                    // forbidden cumulative SQL (e.g. a non-allowlisted
                    // aggregator) would be silently materialised as a plain
                    // full refresh whenever no event-time window is supplied.
                    crate::cumulative::classify_cumulative_sql(
                        &plan.name,
                        &clean_sql,
                        &source_timeseries,
                    )?;
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
                            anyhow::anyhow!(
                                "Failed to create cumulative model {}: {}",
                                plan.name,
                                err
                            )
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
            };

            let exec_result = match exec_result {
                Ok(r) => r,
                Err(e) => {
                    reporter.run_failed(&run_id, Some(&plan.name), &e.to_string());
                    return Err(e);
                }
            };

            total_rows = exec_result.row_count;
            total_rows_overall += exec_result.row_count;
            manifest.models.insert(
                plan.name.clone(),
                ModelRunRecord {
                    strategy: "cumulative_aggregate".to_string(),
                    time_range: match (start_date, end_date) {
                        (Some(s), Some(e)) => Some(TimeRangeRecord {
                            start: s.format("%Y-%m-%d").to_string(),
                            end: e.format("%Y-%m-%d").to_string(),
                        }),
                        _ => None,
                    },
                    partitions_updated: vec![],
                    row_count: exec_result.row_count,
                    duration_ms: model_start.elapsed().as_millis() as u64,
                    batch_safety: Some("cumulative".to_string()),
                },
            );
            reporter.model_completed(&run_id, &plan.name, total_rows, model_start.elapsed());
            continue;
        }

        let result: Result<()> = match plan.incremental.as_ref().filter(|_| !force_full_refresh) {
            Some(inc_plan) => {
                let resolved_strategy = backend.resolve_strategy(&inc_plan.config);

                for (batch_idx, batch) in inc_plan.batches.iter().enumerate() {
                    if cancel.is_cancelled() {
                        reporter.run_cancelled(&run_id);
                        return Ok(build_outcome(
                            &run_id,
                            run_start,
                            None,
                            manifest,
                            total_rows_overall,
                        ));
                    }

                    let batch_start_time = Instant::now();

                    let clean_sql = smelt_parser::strip_frontmatter(&plan.sql);
                    let time_range = TimeRange {
                        start: batch.filter_start.format("%Y-%m-%d").to_string(),
                        end: batch.filter_end.format("%Y-%m-%d").to_string(),
                    };
                    let filtered_sql = inject_time_filter(
                        &clean_sql,
                        &inc_plan.timeseries.event_time_column,
                        &time_range,
                    )?;

                    let compiler = compilers.get(model_target);
                    let resolver = &ephemeral_resolvers[model_target];
                    let compiled = compiler.compile_with_sql_and_ephemerals(
                        &plan.model_file,
                        schema,
                        &filtered_sql,
                        resolver,
                    )?;

                    // The DELETE range must cover the full set of partitions the
                    // INSERT actually writes. `inject_time_filter` clamps the output
                    // on `event_time_column` to [filter_start, filter_end) — i.e. the
                    // run window widened backward by `context_days` (the derived
                    // lookback). For models whose write window spans more than the run
                    // window (a Form B output rebasing, e.g. a session that started on
                    // D-1 and is updated by events on D), using the un-widened
                    // partition_start here would DELETE only the run-window partition
                    // while the INSERT writes the lookback partition too, accumulating
                    // duplicates across consecutive day-by-day runs. Deleting the same
                    // [filter_start, filter_end) the output is clamped to keeps the
                    // DELETE+INSERT contract idempotent regardless of write-window width.
                    let partition = PartitionRange {
                        column: inc_plan.timeseries.partition_column.clone(),
                        start: batch.filter_start.format("%Y-%m-%d").to_string(),
                        end: batch.filter_end.format("%Y-%m-%d").to_string(),
                    };

                    let strategy = MaterializationStrategy::Incremental {
                        partition,
                        strategy: resolved_strategy.clone(),
                        unique_key: inc_plan.config.unique_key.clone(),
                    };

                    let exec_result = backend
                        .execute_model_incremental(
                            schema,
                            &plan.name,
                            &compiled.sql,
                            Materialization::Table,
                            strategy,
                            false,
                        )
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?;

                    total_rows += exec_result.row_count;
                    total_rows_overall += exec_result.row_count;

                    let batch_duration = batch_start_time.elapsed();
                    reporter.batch_completed(
                        &run_id,
                        &plan.name,
                        batch_idx,
                        inc_plan.batches.len(),
                        exec_result.row_count,
                        batch_duration,
                    );
                }

                // Manifest entry for the model
                let (start_str, end_str) = match (start_date, end_date) {
                    (Some(s), Some(e)) => (
                        s.format("%Y-%m-%d").to_string(),
                        e.format("%Y-%m-%d").to_string(),
                    ),
                    _ => (String::new(), String::new()),
                };
                manifest.models.insert(
                    plan.name.clone(),
                    ModelRunRecord {
                        strategy: format!("{:?}", resolved_strategy).to_lowercase(),
                        time_range: Some(TimeRangeRecord {
                            start: start_str.clone(),
                            end: end_str.clone(),
                        }),
                        partitions_updated: vec![],
                        row_count: total_rows,
                        duration_ms: model_start.elapsed().as_millis() as u64,
                        batch_safety: Some("incremental".to_string()),
                    },
                );

                // Update interval store
                if let Ok(mut interval_store) = file_store.load_intervals() {
                    let model_hash = compute_model_hash(&plan.sql);
                    let intervals = interval_store.get_or_create(&plan.name, &model_hash);
                    intervals.record_interval(&start_str, &end_str);
                    let _ = file_store.save_intervals(&interval_store);
                }

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

                let mat = match plan.materialization {
                    smelt_core::config::Materialization::Table => Materialization::Table,
                    smelt_core::config::Materialization::View => Materialization::View,
                    smelt_core::config::Materialization::MaterializedView => {
                        Materialization::MaterializedView
                    }
                    smelt_core::config::Materialization::Ephemeral => {
                        unreachable!("Ephemeral models should be inlined as CTEs, not executed")
                    }
                    smelt_core::config::Materialization::Test => {
                        unreachable!("Test models should not be executed directly")
                    }
                    smelt_core::config::Materialization::CumulativeAggregate => {
                        unreachable!("cumulative_aggregate dispatched above the match")
                    }
                };

                let exec_result = backend
                    .execute_model(schema, &plan.name, &compiled.sql, mat, false)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;

                total_rows = exec_result.row_count;
                total_rows_overall += exec_result.row_count;

                manifest.models.insert(
                    plan.name.clone(),
                    ModelRunRecord {
                        strategy: "full_refresh".to_string(),
                        time_range: None,
                        partitions_updated: vec![],
                        row_count: exec_result.row_count,
                        duration_ms: model_start.elapsed().as_millis() as u64,
                        batch_safety: None,
                    },
                );

                Ok(())
            }
        };

        if let Err(e) = result {
            reporter.run_failed(&run_id, Some(&plan.name), &e.to_string());
            return Err(e);
        }

        let model_duration = model_start.elapsed();
        reporter.model_completed(&run_id, &plan.name, total_rows, model_duration);
    }

    manifest.completed_at = Some(Utc::now());
    if let Err(e) = file_store.save_run(&manifest) {
        tracing::warn!("Failed to save run manifest: {}", e);
    }

    let total_duration: StdDuration = execution_start.elapsed();
    reporter.run_completed(&run_id, total_rows_overall, total_duration);

    Ok(build_outcome(
        &run_id,
        run_start,
        Some(Utc::now()),
        manifest,
        total_rows_overall,
    ))
}

fn build_outcome(
    run_id: &str,
    started_at: chrono::DateTime<Utc>,
    completed_at: Option<chrono::DateTime<Utc>>,
    manifest: RunManifest,
    total_rows: usize,
) -> RunOutcome {
    RunOutcome {
        run_id: run_id.to_string(),
        started_at,
        completed_at,
        models: manifest.models,
        total_rows,
    }
}
