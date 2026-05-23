use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate, Utc};
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;

use smelt_backend::{Backend, Materialization, MaterializationStrategy, PartitionRange};
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::SourcesConfig;
use smelt_planner::{analyze_batch_safety, BatchSafety, Frontmatter, ModelInfo};
use smelt_state::file_store::FileStore;
use smelt_state::intervals::compute_model_hash;
use smelt_state::{generate_run_id, ModelRunRecord, RunManifest, TimeRangeRecord};

use crate::types::*;

/// Manages run execution state. Only one run at a time.
pub struct RunManager {
    inner: Mutex<RunManagerInner>,
    event_tx: broadcast::Sender<RunProgressEvent>,
    project_dir: PathBuf,
}

struct RunManagerInner {
    state: RunState,
    run_id: Option<String>,
    current_model: Option<String>,
    models_completed: usize,
    models_total: usize,
    batches_completed: usize,
    batches_total: usize,
    cancel_token: Option<CancellationToken>,
}

impl RunManager {
    pub fn new(event_tx: broadcast::Sender<RunProgressEvent>, project_dir: PathBuf) -> Self {
        Self {
            inner: Mutex::new(RunManagerInner {
                state: RunState::Idle,
                run_id: None,
                current_model: None,
                models_completed: 0,
                models_total: 0,
                batches_completed: 0,
                batches_total: 0,
                cancel_token: None,
            }),
            event_tx,
            project_dir,
        }
    }

    pub async fn status(&self) -> RunStatusResponse {
        let inner = self.inner.lock().await;
        RunStatusResponse {
            state: inner.state,
            run_id: inner.run_id.clone(),
            current_model: inner.current_model.clone(),
            models_completed: if inner.state == RunState::Running {
                Some(inner.models_completed)
            } else {
                None
            },
            models_total: if inner.state == RunState::Running {
                Some(inner.models_total)
            } else {
                None
            },
            batches_completed: if inner.state == RunState::Running {
                Some(inner.batches_completed)
            } else {
                None
            },
            batches_total: if inner.state == RunState::Running {
                Some(inner.batches_total)
            } else {
                None
            },
        }
    }

    /// Start a run. Returns Err if already running.
    pub async fn execute(
        self: &Arc<Self>,
        request: RunExecuteRequest,
        config: Arc<Config>,
        sources: Arc<Option<SourcesConfig>>,
        graph: Arc<Mutex<DependencyGraph>>,
        db: Arc<Mutex<smelt_db::Database>>,
    ) -> Result<String, &'static str> {
        let mut inner = self.inner.lock().await;
        if inner.state == RunState::Running {
            return Err("A run is already in progress");
        }

        let run_id = generate_run_id();
        let cancel_token = CancellationToken::new();

        inner.state = RunState::Running;
        inner.run_id = Some(run_id.clone());
        inner.current_model = None;
        inner.models_completed = 0;
        inner.models_total = 0;
        inner.batches_completed = 0;
        inner.batches_total = 0;
        inner.cancel_token = Some(cancel_token.clone());

        drop(inner);

        let manager = self.clone();
        let run_id_clone = run_id.clone();
        tokio::spawn(async move {
            let result = manager
                .run_execution(
                    run_id_clone.clone(),
                    request,
                    config,
                    sources,
                    graph,
                    db,
                    cancel_token,
                )
                .await;

            let mut inner = manager.inner.lock().await;
            inner.state = RunState::Idle;
            inner.cancel_token = None;

            if let Err(e) = result {
                tracing::error!("Run {} failed: {}", run_id_clone, e);
            }
        });

        Ok(run_id)
    }

    pub async fn cancel(&self) -> bool {
        let inner = self.inner.lock().await;
        if let Some(ref token) = inner.cancel_token {
            token.cancel();
            true
        } else {
            false
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_execution(
        self: &Arc<Self>,
        run_id: String,
        request: RunExecuteRequest,
        config: Arc<Config>,
        _sources: Arc<Option<SourcesConfig>>,
        graph: Arc<Mutex<DependencyGraph>>,
        db: Arc<Mutex<smelt_db::Database>>,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        let run_start = Utc::now();
        let execution_start = Instant::now();

        if !config.targets.contains_key(&request.target) {
            anyhow::bail!("Target '{}' not found", request.target);
        }

        // Resolve select/exclude into execution order via the shared
        // selection pass. This is the Run Pipeline Parity Rule in action —
        // CLI and UI both call `smelt_runtime::select_executable_models`,
        // so test models, generator files, and target-assignment logic
        // can never drift between consumers.
        let graph_lock = graph.lock().await;

        let selection_request = smelt_runtime::SelectionRequest {
            select: request.select.clone(),
            exclude: request.exclude.clone(),
            target: request.target.clone(),
        };
        let selection =
            smelt_runtime::select_executable_models(&graph_lock, &config, &selection_request)?;
        let selected = selection.ordered_models;
        let target_assignments = selection.target_assignments;
        let cross_edges = selection.cross_engine_edges;
        if !cross_edges.is_empty() {
            tracing::info!(
                "Cross-engine references detected ({} transfer(s) via Parquet)",
                cross_edges.len()
            );
        }

        // Create backends for all needed targets
        let needed_targets: HashSet<String> = target_assignments.values().cloned().collect();
        let mut backends: HashMap<String, Box<dyn Backend>> = HashMap::new();
        for target_name in &needed_targets {
            let target_config = config
                .targets
                .get(target_name)
                .ok_or_else(|| anyhow::anyhow!("Target '{}' not found", target_name))?;
            let backend = create_backend(target_name, target_config, &self.project_dir).await?;
            backends.insert(target_name.clone(), backend);
        }

        // Parse time range
        let start_date = NaiveDate::parse_from_str(&request.start, "%Y-%m-%d")
            .with_context(|| format!("Invalid start date: {}", request.start))?;
        let end_date = NaiveDate::parse_from_str(&request.end, "%Y-%m-%d")
            .with_context(|| format!("Invalid end date: {}", request.end))?;

        if start_date >= end_date {
            anyhow::bail!("Start date must be before end date");
        }

        // Build model execution plans
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

            match (inc_config, ts_config) {
                (Some(inc), Some(ts)) => {
                    let model_info = ModelInfo {
                        name: model_name.clone(),
                        sql: model.content.clone(),
                        refs: model.refs.iter().map(|r| r.model_name.clone()).collect(),
                        timeseries_config: Some(ts.clone()),
                        incremental_config: Some(inc.clone()),
                    };
                    let safety = analyze_batch_safety(&model_info);

                    let (batch_days, context_days) = if request.per_partition {
                        (granularity_days(&ts.granularity), 0)
                    } else if let Some(override_days) = request.batch_size_days {
                        let ctx = match &safety {
                            BatchSafety::BoundedSafe { context_days, .. } => *context_days,
                            _ => 0,
                        };
                        (override_days, ctx)
                    } else {
                        match &safety {
                            BatchSafety::FullyBatchSafe => {
                                ((end_date - start_date).num_days() as u32, 0)
                            }
                            BatchSafety::BoundedSafe {
                                max_chunk_days,
                                context_days,
                                ..
                            } => (*max_chunk_days, *context_days),
                            BatchSafety::PerPartitionOnly { .. } => {
                                (granularity_days(&ts.granularity), 0)
                            }
                        }
                    };

                    let mut batches = Vec::new();
                    let mut batch_start = start_date;
                    while batch_start < end_date {
                        let batch_end =
                            (batch_start + Duration::days(batch_days as i64)).min(end_date);
                        let filter_start = batch_start - Duration::days(context_days as i64);
                        batches.push(BatchPlan {
                            partition_start: batch_start,
                            partition_end: batch_end,
                            filter_start,
                            filter_end: batch_end,
                        });
                        batch_start = batch_end;
                    }

                    total_batches += batches.len();
                    model_plans.push(ModelPlan {
                        name: model_name.clone(),
                        sql: model.content.clone(),
                        materialization: config
                            .get_materialization_with_metadata(model_name, metadata),
                        incremental: Some(IncrementalPlan {
                            config: inc,
                            timeseries: ts,
                            batches,
                        }),
                        model_file: model.clone(),
                    });
                }
                (Some(_inc), None) => {
                    // incremental without timeseries — skip with warning
                    eprintln!(
                        "warning: model '{}' has incremental: but no timeseries: — skipping incremental execution",
                        model_name
                    );
                    model_plans.push(ModelPlan {
                        name: model_name.clone(),
                        sql: model.content.clone(),
                        materialization: config
                            .get_materialization_with_metadata(model_name, metadata),
                        incremental: None,
                        model_file: model.clone(),
                    });
                }
                (None, _) => {
                    model_plans.push(ModelPlan {
                        name: model_name.clone(),
                        sql: model.content.clone(),
                        materialization: config
                            .get_materialization_with_metadata(model_name, metadata),
                        incremental: None,
                        model_file: model.clone(),
                    });
                }
            }
        }

        // Collect inputs needed to build the shared compile pipeline.
        // `all_models` feeds UpstreamSchemas; `ephemeral_models_by_target`
        // feeds the per-target EphemeralResolver. Done inside the graph_lock
        // scope so we don't have to re-acquire it.
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

        // Drop graph lock before execution
        drop(graph_lock);

        // Build the shared compile pipeline. The CompilerRegistry holds one
        // SqlCompiler per needed target; UpstreamSchemas and FnBodyMap are
        // shared across compilers so type-cast wrapping and `smelt.fn.*`
        // expansion work identically across backends. This mirrors what
        // smelt-cli's `commands/run.rs` does so both consumers produce the
        // same SQL — the Run Pipeline Parity Rule's load-bearing guarantee.
        let needed_target_configs: HashMap<String, smelt_core::config::Target> = needed_targets
            .iter()
            .filter_map(|t| config.targets.get(t).map(|c| (t.clone(), c.clone())))
            .collect();
        let mut compilers =
            smelt_runtime::CompilerRegistry::new(config.as_ref(), &needed_target_configs);

        let (upstream_schemas, fn_bodies) = {
            let db_guard = db.lock().await;
            let db_ref: &smelt_db::Database = &db_guard;
            let workspace = smelt_db::Workspace::try_get(db_ref)
                .ok_or_else(|| anyhow::anyhow!("workspace not initialised in smelt-ui DB"))?;
            let upstream = smelt_runtime::UpstreamSchemas::from_database(
                db_ref,
                &self.project_dir,
                &all_models,
            )?;
            let bodies = smelt_runtime::build_fn_body_map(db_ref, workspace);
            (Arc::new(upstream), bodies)
        };
        compilers.set_upstream_schemas_all(upstream_schemas);
        if !fn_bodies.is_empty() {
            compilers.set_function_bodies_all(fn_bodies);
        }

        // Per-target ephemeral resolvers. Empty resolver when the target has
        // no ephemerals — same as the CLI's behaviour.
        let mut ephemeral_resolvers: HashMap<String, smelt_runtime::EphemeralResolver> =
            HashMap::new();
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

        // Update totals
        {
            let mut inner = self.inner.lock().await;
            inner.models_total = model_plans.len();
            inner.batches_total = total_batches;
        }

        // Send run_started event
        let _ = self.event_tx.send(RunProgressEvent::RunStarted {
            run_id: run_id.clone(),
            models: model_plans.iter().map(|m| m.name.clone()).collect(),
            total_batches,
        });

        let file_store = FileStore::new(&self.project_dir);
        let mut manifest = RunManifest {
            run_id: run_id.clone(),
            started_at: run_start,
            completed_at: None,
            models: std::collections::HashMap::new(),
        };

        // Execute models
        for (model_idx, plan) in model_plans.iter().enumerate() {
            // Check cancellation
            if cancel_token.is_cancelled() {
                let _ = self.event_tx.send(RunProgressEvent::RunCancelled {
                    run_id: run_id.clone(),
                });
                return Ok(());
            }

            // Update state
            {
                let mut inner = self.inner.lock().await;
                inner.current_model = Some(plan.name.clone());
            }

            let _ = self.event_tx.send(RunProgressEvent::ModelStarted {
                run_id: run_id.clone(),
                model: plan.name.clone(),
                model_index: model_idx,
                models_total: model_plans.len(),
            });

            let model_start = Instant::now();
            let mut total_rows = 0usize;

            let model_target = &target_assignments[&plan.name];
            let backend = backends[model_target].as_ref();
            let schema = &config.targets[model_target].schema;

            let result: Result<()> = match &plan.incremental {
                Some(inc_plan) => {
                    let resolved_strategy = backend.resolve_strategy(&inc_plan.config);

                    for (batch_idx, batch) in inc_plan.batches.iter().enumerate() {
                        if cancel_token.is_cancelled() {
                            let _ = self.event_tx.send(RunProgressEvent::RunCancelled {
                                run_id: run_id.clone(),
                            });
                            return Ok(());
                        }

                        let batch_start_time = Instant::now();

                        // Compile SQL: strip frontmatter, inject time filter, resolve refs
                        let clean_sql = smelt_parser::strip_frontmatter(&plan.sql);
                        let time_range = smelt_runtime::TimeRange {
                            start: batch.filter_start.format("%Y-%m-%d").to_string(),
                            end: batch.filter_end.format("%Y-%m-%d").to_string(),
                        };
                        let filtered_sql = smelt_runtime::inject_time_filter(
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
                        let compiled_sql = compiled.sql;

                        let partition = PartitionRange {
                            column: inc_plan.timeseries.partition_column.clone(),
                            start: batch.partition_start.format("%Y-%m-%d").to_string(),
                            end: batch.partition_end.format("%Y-%m-%d").to_string(),
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
                                &compiled_sql,
                                Materialization::Table,
                                strategy,
                                false,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("{}", e))?;

                        total_rows += exec_result.row_count;

                        let batch_duration = batch_start_time.elapsed();
                        {
                            let mut inner = self.inner.lock().await;
                            inner.batches_completed += 1;
                        }

                        let _ = self.event_tx.send(RunProgressEvent::BatchCompleted {
                            run_id: run_id.clone(),
                            model: plan.name.clone(),
                            batch_index: batch_idx,
                            batches_total: inc_plan.batches.len(),
                            row_count: exec_result.row_count,
                            duration_ms: batch_duration.as_millis() as u64,
                        });
                    }

                    // Record manifest entry
                    manifest.models.insert(
                        plan.name.clone(),
                        ModelRunRecord {
                            strategy: format!("{:?}", resolved_strategy).to_lowercase(),
                            time_range: Some(TimeRangeRecord {
                                start: request.start.clone(),
                                end: request.end.clone(),
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
                        intervals.record_interval(&request.start, &request.end);
                        let _ = file_store.save_intervals(&interval_store);
                    }

                    Ok(())
                }
                None => {
                    // Full refresh: compile and execute
                    let clean_sql = smelt_parser::strip_frontmatter(&plan.sql);
                    let compiler = compilers.get(model_target);
                    let resolver = &ephemeral_resolvers[model_target];
                    let compiled = compiler.compile_with_sql_and_ephemerals(
                        &plan.model_file,
                        schema,
                        &clean_sql,
                        resolver,
                    )?;
                    let compiled_sql = compiled.sql;

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
                            return Err(anyhow::anyhow!(
                                "cumulative_aggregate models require an incremental run window — \
                                 use `smelt run --event-time-start … --event-time-end …` from the CLI"
                            ));
                        }
                    };

                    let exec_result = backend
                        .execute_model(schema, &plan.name, &compiled_sql, mat, false)
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?;

                    total_rows = exec_result.row_count;

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
                let _ = self.event_tx.send(RunProgressEvent::RunFailed {
                    run_id: run_id.clone(),
                    error: e.to_string(),
                    model: Some(plan.name.clone()),
                });
                return Err(e);
            }

            let model_duration = model_start.elapsed();

            {
                let mut inner = self.inner.lock().await;
                inner.models_completed += 1;
            }

            let _ = self.event_tx.send(RunProgressEvent::ModelCompleted {
                run_id: run_id.clone(),
                model: plan.name.clone(),
                row_count: total_rows,
                duration_ms: model_duration.as_millis() as u64,
            });
        }

        // Save manifest
        manifest.completed_at = Some(Utc::now());
        if let Err(e) = file_store.save_run(&manifest) {
            tracing::warn!("Failed to save run manifest: {}", e);
        }

        let total_duration = execution_start.elapsed();
        let _ = self.event_tx.send(RunProgressEvent::RunCompleted {
            run_id: run_id.clone(),
            models_executed: model_plans.len(),
            duration_ms: total_duration.as_millis() as u64,
        });

        Ok(())
    }
}

// --- Internal types ---

struct ModelPlan {
    name: String,
    sql: String,
    materialization: smelt_core::config::Materialization,
    incremental: Option<IncrementalPlan>,
    /// The original `ModelFile` from the dependency graph — needed by
    /// `SqlCompiler::compile_with_sql_and_ephemerals` to derive the
    /// materialization, address-based DB name, and metadata-aware overrides.
    model_file: smelt_core::ModelFile,
}

struct IncrementalPlan {
    config: smelt_core::IncrementalConfig,
    timeseries: smelt_core::config::TimeseriesConfig,
    batches: Vec<BatchPlan>,
}

struct BatchPlan {
    partition_start: NaiveDate,
    partition_end: NaiveDate,
    filter_start: NaiveDate,
    filter_end: NaiveDate,
}

// --- Helpers ---

fn granularity_days(g: &smelt_core::Granularity) -> u32 {
    match g {
        smelt_core::Granularity::Hour => 1,
        smelt_core::Granularity::Day => 1,
        smelt_core::Granularity::Week => 7,
        smelt_core::Granularity::Month => 30,
        smelt_core::Granularity::Quarter => 91,
        smelt_core::Granularity::Year => 365,
    }
}

#[allow(unreachable_code, unused_variables)]
async fn create_backend(
    target_name: &str,
    target_config: &smelt_core::config::Target,
    project_dir: &Path,
) -> Result<Box<dyn Backend>> {
    use smelt_core::config::BackendType;
    match target_config.backend_type() {
        BackendType::DuckDB => {
            #[cfg(feature = "duckdb")]
            {
                let database = target_config
                    .database
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("DuckDB target requires 'database' field"))?;
                let db_path = project_dir.join(database);
                let backend =
                    smelt_backend_duckdb::DuckDbBackend::new(&db_path, &target_config.schema)
                        .await
                        .with_context(|| format!("Failed to initialize DuckDB at {:?}", db_path))?;
                Ok(Box::new(backend))
            }
            #[cfg(not(feature = "duckdb"))]
            {
                anyhow::bail!("DuckDB feature not enabled")
            }
        }
        BackendType::Spark => {
            anyhow::bail!("Spark backend not yet supported in UI mode")
        }
    }
}
