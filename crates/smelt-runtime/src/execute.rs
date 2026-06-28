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
//! incremental batches, and cumulative dispatch via `crate::cumulative`),
//! cancellation handling, manifest writes, and interval-store updates.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use smelt_backend::{Backend, Materialization, MaterializationStrategy, PartitionRange};
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_planner::Frontmatter;
use smelt_state::file_store::FileStore;
use smelt_state::intervals::compute_model_hash;
use smelt_state::{ModelRunRecord, RunManifest, TimeRangeRecord};

use crate::check_runner::{run_single_check, CheckOutcome, CheckStatus};
use crate::compile::build_source_bound_map;
use crate::compile::CompilerRegistry;
use crate::reporter::RunReporter;
use crate::safety::{build_model_graph, check_bound_derivation, check_planner_safety};
use crate::schema_evolution::{
    check_and_migrate, ddl_backend_for_dialect, extract_evolution_maps, infer_deployed_columns,
};
use crate::select::{select_executable_models, SelectionRequest};
use crate::transformer::{inject_source_filters, inject_time_filter, TimeRange};
use crate::types::{ExecuteRequest, ModelPlanRecord, ModelStrategy, PlanSummary, RunOutcome};
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
        let model_graph = build_model_graph(&selected, &graph_lock, &config);
        check_planner_safety(&model_graph, request.enforce_safety)?;
        check_bound_derivation(&model_graph, request.enforce_safety)?;
    }

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
                        .cloned()
                        .or_else(|| frontmatter.as_ref().and_then(|f| f.incremental.clone()));
                    let ts_config = config
                        .get_timeseries_with_metadata(model_name, metadata)
                        .cloned()
                        .or_else(|| metadata.and_then(|m| m.timeseries.clone()));

                    // Route cumulative detection through is_cumulative().
                    if metadata.is_some_and(|m| m.is_cumulative()) {
                        ModelStrategy::Cumulative
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
                    compiler.build_ephemeral_resolver(models_slice, schema),
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
                let sql_to_emit = match compiler
                    .compile_with_sql_and_ephemerals(model_file, schema, &clean_sql, resolver)
                {
                    Ok(compiled) => compiled.sql,
                    Err(_) => String::new(),
                };
                reporter.model_compiled(&run_id, model_name, &sql_to_emit);
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

                if let Some(ref warning) = inc_windows.wide_batch_warning {
                    warn!("model '{model_name}': {warning}");
                }

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
                warn!(
                    "model '{model_name}' has incremental: but no timeseries: — skipping incremental execution"
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
    // Project-wide `smelt.<path> → timeseries` map. Merges model-frontmatter
    // timeseries with per-entity source YAML timeseries declarations (BUG-072).
    // Cumulative dispatch and incremental pushdown (Phase 3) both use this map.
    let source_infos = smelt_core::discover_source_infos(project_dir, &config.paths);
    let source_timeseries = build_source_timeseries_map(&graph_lock, &source_infos);
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

    // Build model → all_upstream map for the selected set (needed for
    // downstream closure computation). Captured here to avoid holding the
    // graph lock across awaits in the model loop.
    let upstream_map: HashMap<String, HashSet<String>> = if request.run_checks {
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
            return Ok(build_outcome(
                &run_id,
                run_start,
                None,
                manifest,
                0,
                check_results,
            ));
        }

        // ── Skip set: skip models downstream of a failed error check ─────
        if request.run_checks && skip_set.contains(&plan.name) {
            tracing::info!(
                "Skipping model '{}' — downstream of a failed error-severity check",
                plan.name
            );
            manifest.models.insert(
                plan.name.clone(),
                smelt_state::ModelRunRecord {
                    strategy: "skipped_failed_check".to_string(),
                    time_range: None,
                    partitions_updated: vec![],
                    row_count: 0,
                    duration_ms: 0,
                    batch_safety: Some("skipped".to_string()),
                },
            );
            reporter.model_completed(&run_id, &plan.name, 0, std::time::Duration::ZERO);
            continue;
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
                        let table_format = config.get_format(
                            &plan.name,
                            plan.model_file.metadata.as_deref(),
                            target_config,
                        );
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
        let plan_is_cumulative = plan
            .model_file
            .metadata
            .as_deref()
            .is_some_and(|m| m.is_cumulative());
        if plan_is_cumulative {
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
            // ── Check seam A: cumulative arm ─────────────────────────────────
            if request.run_checks {
                let (outcomes, to_skip) = run_model_checks(
                    &plan.name,
                    &checks_by_model,
                    &compilers,
                    &backends,
                    &target_assignments,
                    &ephemeral_resolvers,
                    config.as_ref(),
                    &upstream_map,
                    &selected,
                    reporter,
                    &run_id,
                )
                .await;
                check_results.extend(outcomes);
                skip_set.extend(to_skip);
            }
            continue;
        }

        let result: Result<()> = match plan.incremental.as_ref().filter(|_| !force_full_refresh) {
            Some(inc_plan) => {
                let resolved_strategy = backend.resolve_strategy(&inc_plan.config);

                // Build source bound map once per model for source-filter pushdown (BUG-073).
                // The model SQL is the same for every batch — compute once and reuse.
                // `source_timeseries` is the project-wide smelt-ref → TimeseriesConfig map
                // (built from model frontmatter + source YAML declarations in Phase 2).
                // We convert it to the dep_timeseries shape that `build_source_bound_map`
                // expects: smelt_ref → (address_segments, partition_column).
                let sql_for_bounds = smelt_parser::strip_frontmatter(&plan.sql);
                let dep_ts: std::collections::HashMap<String, (Vec<String>, String)> =
                    source_timeseries
                        .iter()
                        .filter_map(|(smelt_ref, ts)| {
                            // Strip the leading "smelt." prefix to get the path segments.
                            let path = smelt_ref.strip_prefix("smelt.")?;
                            let segs: Vec<String> = path.split('.').map(String::from).collect();
                            Some((smelt_ref.clone(), (segs, ts.partition_column.clone())))
                        })
                        .collect();
                let per_model_source_bounds = build_source_bound_map(&sql_for_bounds, &dep_ts);

                for (batch_idx, batch) in inc_plan.batches.iter().enumerate() {
                    if cancel.is_cancelled() {
                        reporter.run_cancelled(&run_id);
                        return Ok(build_outcome(
                            &run_id,
                            run_start,
                            None,
                            manifest,
                            total_rows_overall,
                            vec![],
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

                    // Source-filter pushdown: narrow each source read to the run window
                    // (partition_start / partition_end) plus per-source bounds derived from
                    // the model SQL's INTERVAL patterns. The run window is the unwidened
                    // partition range — `inject_time_filter` above uses filter_start/filter_end
                    // (the widened write window) for the model's own output constraint; source
                    // filters derive from the run window so the source scan tracks the
                    // partition being produced, not the potentially wider DELETE range.
                    let run_range = TimeRange {
                        start: batch.partition_start.format("%Y-%m-%d").to_string(),
                        end: batch.partition_end.format("%Y-%m-%d").to_string(),
                    };
                    let filtered_sql =
                        inject_source_filters(&filtered_sql, &per_model_source_bounds, &run_range);

                    let compiler = compilers.get(model_target);
                    let resolver = &ephemeral_resolvers[model_target];
                    let compiled = compiler.compile_with_sql_and_ephemerals(
                        &plan.model_file,
                        schema,
                        &filtered_sql,
                        resolver,
                    )?;
                    reporter.model_compiled(&run_id, &plan.name, &compiled.sql);

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
                            &plan.model_file.db_name_owned(),
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
                reporter.model_compiled(&run_id, &plan.name, &compiled.sql);

                let mat = match plan.materialization {
                    smelt_core::config::Materialization::Table => Materialization::Table,
                    smelt_core::config::Materialization::View => Materialization::View,
                    smelt_core::config::Materialization::MaterializedView => {
                        Materialization::MaterializedView
                    }
                    smelt_core::config::Materialization::Ephemeral => {
                        unreachable!("Ephemeral models should be inlined as CTEs, not executed")
                    }
                };

                let exec_result = backend
                    .execute_model(
                        schema,
                        &plan.model_file.db_name_owned(),
                        &compiled.sql,
                        mat,
                        false,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;

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
                            &file_store,
                            &db_table_name,
                            &plan.sql,
                            &inferred_columns,
                            existing_version,
                        ) {
                            tracing::warn!(
                                "Failed to save deployed schema for '{}': {}",
                                plan.name,
                                e
                            );
                        }
                    }
                }

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
        // ── Check seam B: incremental / full-refresh arm ─────────────────────
        if request.run_checks {
            let (outcomes, to_skip) = run_model_checks(
                &plan.name,
                &checks_by_model,
                &compilers,
                &backends,
                &target_assignments,
                &ephemeral_resolvers,
                config.as_ref(),
                &upstream_map,
                &selected,
                reporter,
                &run_id,
            )
            .await;
            check_results.extend(outcomes);
            skip_set.extend(to_skip);
        }
    }

    manifest.completed_at = Some(Utc::now());
    if let Err(e) = file_store.save_run(&manifest) {
        tracing::warn!("Failed to save run manifest: {}", e);
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
    reporter.run_completed(&run_id, total_rows_overall, total_duration);

    Ok(build_outcome(
        &run_id,
        run_start,
        Some(Utc::now()),
        manifest,
        total_rows_overall,
        check_results,
    ))
}

fn build_outcome(
    run_id: &str,
    started_at: chrono::DateTime<Utc>,
    completed_at: Option<chrono::DateTime<Utc>>,
    manifest: RunManifest,
    total_rows: usize,
    check_results: Vec<CheckOutcome>,
) -> RunOutcome {
    RunOutcome {
        run_id: run_id.to_string(),
        started_at,
        completed_at,
        models: manifest.models,
        total_rows,
        plan_summary: None,
        check_results,
    }
}

/// Execute all checks registered for `model_name` after it materializes.
///
/// Returns `(outcomes, models_to_skip)` where:
/// - `outcomes` is the per-check result list to append to `check_results`
/// - `models_to_skip` is the downstream closure to add to `skip_set` when an
///   error-severity check fails (derived from `upstream_map`)
#[allow(clippy::too_many_arguments)]
async fn run_model_checks(
    model_name: &str,
    checks_by_model: &HashMap<String, Vec<smelt_core::ModelFile>>,
    compilers: &CompilerRegistry,
    backends: &HashMap<String, Box<dyn Backend>>,
    target_assignments: &HashMap<String, String>,
    ephemeral_resolvers: &HashMap<String, EphemeralResolver>,
    config: &smelt_core::config::Config,
    upstream_map: &HashMap<String, HashSet<String>>,
    selected: &[String],
    reporter: &dyn RunReporter,
    run_id: &str,
) -> (Vec<CheckOutcome>, HashSet<String>) {
    use smelt_core::metadata::CheckSeverity;

    let Some(check_files) = checks_by_model.get(model_name) else {
        return (vec![], HashSet::new());
    };

    let model_target = target_assignments
        .get(model_name)
        .map(|s| s.as_str())
        .unwrap_or(model_name);

    let Some(backend) = backends.get(model_target) else {
        return (vec![], HashSet::new());
    };

    let schema = &config.targets[model_target].schema;
    let compiler = compilers.get(model_target);

    static EMPTY_RESOLVER: std::sync::OnceLock<EphemeralResolver> = std::sync::OnceLock::new();
    let resolver = ephemeral_resolvers
        .get(model_target)
        .unwrap_or_else(|| EMPTY_RESOLVER.get_or_init(EphemeralResolver::empty));

    let ephemeral_names = &resolver.ephemeral_names;

    let mut outcomes: Vec<CheckOutcome> = Vec::new();
    let mut any_error_check_failed = false;

    for check_model in check_files {
        let severity: CheckSeverity = check_model
            .metadata
            .as_ref()
            .and_then(|m| m.check.as_ref())
            .map(|c| c.severity.clone())
            .unwrap_or_default();

        let outcome = match run_single_check(
            compiler,
            backend.as_ref(),
            schema,
            check_model,
            severity,
            ephemeral_names,
            resolver,
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("check '{}' error: {}", check_model.name, e);
                CheckOutcome {
                    name: check_model.name.clone(),
                    severity: CheckSeverity::Error,
                    status: CheckStatus::Fail,
                    row_count: 0,
                    sample: vec![],
                    message: Some(e.to_string()),
                    sql: None,
                }
            }
        };

        let status_str = match outcome.status {
            CheckStatus::Pass => "pass",
            CheckStatus::Fail => "fail",
            CheckStatus::Warn => "warn",
            CheckStatus::TargetNotBuilt => "target_not_built",
        };

        reporter.check_result(run_id, &outcome.name, status_str, outcome.row_count);

        if matches!(
            (&outcome.severity, &outcome.status),
            (
                CheckSeverity::Error,
                CheckStatus::Fail | CheckStatus::TargetNotBuilt
            )
        ) {
            any_error_check_failed = true;
        }

        outcomes.push(outcome);
    }

    // Compute downstream closure to skip (only for error-severity failures).
    let models_to_skip: HashSet<String> = if any_error_check_failed {
        selected
            .iter()
            .filter(|m| {
                upstream_map
                    .get(*m)
                    .is_some_and(|ups| ups.contains(model_name))
            })
            .cloned()
            .collect()
    } else {
        HashSet::new()
    };

    (outcomes, models_to_skip)
}

/// Build the project-wide `smelt.<path> → timeseries` lookup map used by
/// the planner (cumulative classification) and the incremental execute path
/// (source-filter pushdown, Phase 3).
///
/// Merges two sources of timeseries declarations:
/// 1. **Model-frontmatter** — an incremental model whose output partitions by
///    a time column is itself a timeseries source for downstream consumers.
/// 2. **Source YAML** — per-entity sources declaring a `timeseries:` block
///    become pushdown candidates for incremental models reading them (BUG-072).
///
/// In valid workspaces a model and a source cannot share the same `smelt.<path>`
/// address (address-uniqueness constraint). If they did, the source YAML entry
/// wins (it is inserted last); that is documented here as a design decision
/// pending a normative spec ruling.
pub fn build_source_timeseries_map(
    graph: &smelt_core::graph::DependencyGraph,
    source_infos: &[smelt_core::SourceInfo],
) -> smelt_planner::SourceTimeseriesMap {
    let mut map = smelt_planner::SourceTimeseriesMap::new();

    // Model-frontmatter entries. `unwrap_or_default`: if the graph is cyclic,
    // `execution_order` would fail, but the caller's planner-safety gate already
    // catches cycles before this function is reached, so the fallback is a
    // degenerate safety net.
    let exec_order = graph.execution_order().unwrap_or_default();
    for model_name in &exec_order {
        let Ok(model) = graph.get_model(model_name) else {
            continue;
        };
        if let Some(ts) = model.metadata.as_deref().and_then(|m| m.timeseries.clone()) {
            map.insert(format!("smelt.{}", model.address_segments.join(".")), ts);
        }
    }

    // Source YAML entries (BUG-072 / Phase 2).
    for source in source_infos {
        if let Some(ts) = &source.timeseries {
            map.insert(
                format!("smelt.{}", source.address_segments.join(".")),
                ts.clone(),
            );
        }
    }

    map
}
