use anyhow::{Context, Result};
use arrow::util::pretty;
use chrono::{NaiveDate, Utc};
use smelt_backend::PartitionSpec;
use smelt_cli::{
    compute_batches_for_model, compute_incremental_windows, discover_python_models, executor,
    find_project_root, init_db, inject_time_filter, migration, parse_selector, BackendRegistry,
    BackfillOptions, CompilerRegistry, Config, LogicalGraph, Materialization, ModelDiscovery,
    PhysicalGraphBuilder, PhysicalStrategy, SourcesConfig, TimeRange,
};
use smelt_core::metadata::SchemaEvolutionStrategy;
use smelt_planner::{Frontmatter, ModelGraph, ModelInfo, Planner};
use smelt_state::file_store::FileStore;
use smelt_state::intervals::compute_model_hash;
use smelt_state::{generate_run_id, ModelRunRecord, RunManifest, TimeRangeRecord};
use std::collections::{HashMap, HashSet};

use tracing::{debug, info, warn};

use crate::helpers::{
    generate_partition_values, granularity_label, infer_deployed_columns, strategy_label,
};
use crate::RunArgs;

pub async fn run(args: RunArgs) -> Result<()> {
    let run_start = Utc::now();

    // 1. Find project root
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    info!("Project directory: {}", project_dir.display());

    // 2. Load configuration
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    info!("Project: {} (version {})", config.name, config.version);

    // Validate default target exists early
    if !config.targets.contains_key(&args.target) {
        return Err(anyhow::anyhow!(
            "Target '{}' not found in smelt.yml. Available targets: {}",
            args.target,
            config
                .targets
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // Load source configuration (optional)
    let sources = SourcesConfig::load(&project_dir).ok();

    if let Some(ref source_config) = sources {
        let source_count: usize = source_config.sources.iter().map(|s| s.tables.len()).sum();
        info!("Loaded {} source tables", source_count);
    }

    // 3. Discover models (SQL + Python)
    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Filter out test models — they shouldn't be materialized
    models.retain(|m| !m.is_test());

    // Discover and execute Python models
    let python_files = discovery
        .discover_python_files()
        .with_context(|| "Failed to scan for Python models")?;

    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .with_context(|| "Failed to discover Python models")?;

        if !python_models.is_empty() {
            info!(
                "Found {} Python model(s) from {} file(s)",
                python_models.len(),
                python_files.len()
            );
            models.extend(python_models);
        }
    }

    if models.is_empty() {
        return Err(anyhow::anyhow!(
            "No models found in model paths: {}",
            config.model_paths.join(", ")
        ));
    }

    info!("Found {} models total", models.len());

    // Report any parse errors
    for model in &models {
        if !model.parse_errors.is_empty() {
            warn!("Parse errors in {}:", model.name);
            for error in &model.parse_errors {
                warn!("  - {} at {:?}", error.message, error.range);
            }
        }
    }

    // Validate materialization configs (e.g. ephemeral + incremental conflicts)
    {
        let metadata_map: HashMap<String, smelt_cli::ModelMetadata> = models
            .iter()
            .filter_map(|m| {
                m.metadata
                    .as_ref()
                    .map(|md| (m.name.clone(), md.as_ref().clone()))
            })
            .collect();
        let config_errors = config.validate_model_configs(&metadata_map);
        if !config_errors.is_empty() {
            for (model, msg) in &config_errors {
                tracing::error!("model '{}': {}", model, msg);
            }
            return Err(anyhow::anyhow!(
                "Model configuration validation failed ({} error(s))",
                config_errors.len()
            ));
        }
    }

    // 4. Build logical graph (eagerly resolves config per model)
    let graph = LogicalGraph::build(models, sources.as_ref(), &config, &args.target)
        .with_context(|| "Failed to build logical graph")?;

    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    graph.warn_unused_ephemerals();

    // 5. Determine execution order (with optional selector filtering)
    let execution_order = if args.select.is_empty() && args.exclude.is_empty() {
        graph
            .execution_order()
            .with_context(|| "Failed to determine execution order")?
    } else {
        let mut selected = if args.select.is_empty() {
            graph.all_model_names()
        } else {
            let selectors: Vec<_> = args
                .select
                .iter()
                .map(|s| parse_selector(s).with_context(|| format!("Invalid selector '{}'", s)))
                .collect::<Result<_, _>>()?;

            // Check if any directly-selected model is ephemeral
            for selector in &selectors {
                if let smelt_cli::SelectionMethod::ModelName(name) = &selector.method {
                    if !selector.include_upstream && !selector.include_downstream {
                        if let Ok(node) = graph.get_node(name) {
                            if node.materialization == Materialization::Ephemeral {
                                return Err(anyhow::anyhow!(
                                    "Cannot run ephemeral model '{}' directly — ephemeral models are inlined as CTEs into downstream models.",
                                    name
                                ));
                            }
                        }
                    }
                }
            }

            graph
                .select_models(&selectors)
                .with_context(|| "Failed to select models")?
        };

        if !args.exclude.is_empty() {
            let excludes: Vec<_> = args
                .exclude
                .iter()
                .map(|s| {
                    parse_selector(s).with_context(|| format!("Invalid exclude selector '{}'", s))
                })
                .collect::<Result<_, _>>()?;

            selected = graph
                .exclude_models(&selected, &excludes)
                .with_context(|| "Failed to apply exclude selectors")?;
        }

        if selected.is_empty() {
            info!("No models matched the selectors");
            return Ok(());
        }

        graph
            .filtered_execution_order(&selected)
            .with_context(|| "Failed to determine execution order")?
    };

    info!(
        "Execution order: {}",
        execution_order
            .iter()
            .enumerate()
            .map(|(i, name)| format!("{}. {}", i + 1, name))
            .collect::<Vec<_>>()
            .join(" → ")
    );

    if args.dry_run {
        info!("[DRY RUN] Skipping execution");
        return Ok(());
    }

    // 6. Create backends for needed targets
    // Log cross-backend references (supported via Parquet exchange)
    let cross_edges = graph.find_cross_backend_edges();
    if !cross_edges.is_empty() {
        info!(
            "Cross-engine references detected ({} transfer(s) via Parquet):",
            cross_edges.len()
        );
        for edge in &cross_edges {
            info!(
                "  {} ({}) -> {} ({})",
                edge.upstream, edge.upstream_target, edge.downstream, edge.downstream_target
            );
        }
    }

    let needed_targets: HashSet<String> = execution_order
        .iter()
        .filter_map(|name| graph.get_node(name).ok())
        .map(|node| node.target.clone())
        .collect();
    let registry = BackendRegistry::new(
        &config.targets,
        &needed_targets,
        &project_dir,
        args.database,
    )
    .await?;

    let needed_target_configs: HashMap<String, _> = needed_targets
        .iter()
        .map(|name| (name.clone(), config.targets[name].clone()))
        .collect();
    let mut compilers = CompilerRegistry::new(&config, &needed_target_configs);

    // Set up cross-engine ref resolution (Parquet exchange)
    // Only resolve edges where the downstream model is in the execution order.
    // Paths are computed from target config (warehouse field) so we don't need
    // the upstream backend to be instantiated (e.g., Spark may not be running).
    if !cross_edges.is_empty() {
        let execution_set: HashSet<&str> = execution_order.iter().map(|s| s.as_str()).collect();
        let mut refs_by_target: HashMap<String, HashMap<String, String>> = HashMap::new();
        for edge in &cross_edges {
            if !execution_set.contains(edge.downstream.as_str()) {
                continue;
            }
            let upstream_target_config = &config.targets[&edge.upstream_target];
            let upstream_schema = &upstream_target_config.schema;
            // Compute materialized path from config: warehouse/schema/model_name
            let rel_path = if let Some(ref warehouse) = upstream_target_config.warehouse {
                Some(std::path::PathBuf::from(format!(
                    "{}/{}/{}",
                    warehouse, upstream_schema, edge.upstream
                )))
            } else if needed_targets.contains(&edge.upstream_target) {
                // Fallback: if backend is available, ask it
                registry
                    .get(&edge.upstream_target)
                    .materialized_path(upstream_schema, &edge.upstream)
            } else {
                None
            };
            if let Some(rel_path) = rel_path {
                let abs_path = project_dir.join(&rel_path);
                let parquet_expr = format!(
                    "read_parquet('{}/**/*.parquet', hive_partitioning=true)",
                    abs_path.display()
                );
                refs_by_target
                    .entry(edge.downstream_target.clone())
                    .or_default()
                    .insert(edge.upstream.clone(), parquet_expr);
            }
        }
        for (target, refs) in refs_by_target {
            compilers.set_cross_engine_refs(&target, refs);
        }
    }

    // 7. Validate sources exist (if sources.yml present)
    // Skip source validation when --select/--exclude is used: the selected models
    // may not reference all sources, and some sources may live on backends that
    // aren't running (e.g., Spark sources when only DuckDB models are selected).
    if let Some(ref source_config) = sources {
        if args.select.is_empty() && args.exclude.is_empty() {
            executor::validate_sources(registry.get(&args.target), source_config)
                .await
                .with_context(|| "Source validation failed")?;
        } else {
            debug!("Skipping source validation (--select/--exclude active)");
        }
    }

    // 8. Parse time range if provided (for incremental processing)
    //    --start/--end are aliases for --event-time-start/--event-time-end
    let effective_start = args.start.as_ref().or(args.event_time_start.as_ref());
    let effective_end = args.end.as_ref().or(args.event_time_end.as_ref());

    let time_range = match (effective_start, effective_end) {
        (Some(start), Some(end)) => {
            // Validate date format
            NaiveDate::parse_from_str(start, "%Y-%m-%d").with_context(|| {
                format!("Invalid start date format: {}. Expected YYYY-MM-DD", start)
            })?;
            NaiveDate::parse_from_str(end, "%Y-%m-%d").with_context(|| {
                format!("Invalid end date format: {}. Expected YYYY-MM-DD", end)
            })?;

            info!("Time range: {} to {} (exclusive)", start, end);
            Some(TimeRange {
                start: start.clone(),
                end: end.clone(),
            })
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(anyhow::anyhow!(
                "Both --start and --end (or --event-time-start and --event-time-end) must be provided together"
            ));
        }
        _ => None,
    };

    // Handle --auto mode: compute time range from interval store
    let time_range = if args.auto && time_range.is_none() {
        let file_store = FileStore::new(&project_dir);
        let interval_store = match file_store.load_intervals() {
            Ok(store) => store,
            Err(e) => {
                warn!(
                    "Failed to load interval store: {}. Starting with empty history.",
                    e
                );
                smelt_state::intervals::IntervalStore::default()
            }
        };
        let today = Utc::now().format("%Y-%m-%d").to_string();

        // Find the latest covered date across all incremental models
        let mut latest: Option<NaiveDate> = None;
        for model_name in &execution_order {
            if let Some(intervals) = interval_store.get(model_name) {
                if let Some(date) = intervals.latest_date() {
                    latest = Some(match latest {
                        Some(prev) => prev.min(date), // use the earliest "latest" — conservative
                        None => date,
                    });
                }
            }
        }

        if let Some(start_date) = latest {
            let start = start_date.format("%Y-%m-%d").to_string();
            info!(
                "[AUTO] Time range: {} to {} (from interval store)",
                start, today
            );
            Some(TimeRange { start, end: today })
        } else {
            info!("[AUTO] No interval history found. Use --start/--end for initial run.");
            None
        }
    } else {
        time_range
    };

    let backfill_options = BackfillOptions {
        batch_size_days: args.batch_size,
        per_partition: args.per_partition,
    };

    // 9. Run optimizer on discovered models
    let mut opt_graph = ModelGraph::new();
    for model_name in &execution_order {
        let model = graph.get_model(model_name)?;
        let frontmatter = Frontmatter::parse(&model.content);
        opt_graph.add_model(ModelInfo {
            name: model.name.clone(),
            sql: model.content.clone(),
            refs: model.refs.iter().map(|r| r.model_name.clone()).collect(),
            incremental_config: frontmatter.as_ref().and_then(|f| f.incremental.clone()),
        });
    }

    let planner = Planner::new();
    let (transformations, plan_errors) = planner.plan(&opt_graph);

    for err in &plan_errors {
        warn!("Planner error: {}", err);
    }

    // 10. Build physical graph (resolves strategies, ephemeral resolvers)
    let target_schemas: HashMap<String, String> = needed_targets
        .iter()
        .map(|name| (name.clone(), registry.target_config(name).schema.clone()))
        .collect();

    let physical_graph = PhysicalGraphBuilder::new(
        &graph,
        &transformations,
        time_range.clone(),
        &compilers,
        target_schemas,
    )
    .build()
    .with_context(|| "Failed to build physical graph")?;

    // Print planner summary
    let planner_summary = physical_graph.planner_summary();
    if !planner_summary.is_empty() {
        info!("Planner:");
        for (model, desc) in &planner_summary {
            info!("  {} -> {}", model, desc);
        }
    }

    // Mandatory time range check: if any model has planner-detected incremental and no time range
    if time_range.is_none() {
        let inc_models: Vec<&str> = physical_graph
            .iter_in_order()
            .filter(|n| matches!(n.strategy, PhysicalStrategy::Incremental { .. }))
            .map(|n| n.name.as_str())
            .collect();
        if !inc_models.is_empty() {
            return Err(anyhow::anyhow!(
                "Incremental models selected for run ({}) but --event-time-start and --event-time-end not provided.\n\
                 These are required when running incremental models, similar to dbt's microbatch parameters.",
                inc_models.join(", ")
            ));
        }
    }

    // 11. Compile and execute each model

    // Initialize type inference DB for schema evolution
    let all_models: Vec<_> = physical_graph
        .iter_in_order()
        .map(|node| &node.model_file)
        .cloned()
        .collect();
    let type_db = init_db(&project_dir, &all_models);
    let file_store = FileStore::new(&project_dir);

    info!("{}", "=".repeat(60));
    info!("Executing models...");
    info!("{}", "=".repeat(60));

    let mut results: Vec<(smelt_backend::ExecutionResult, PhysicalStrategy)> = Vec::new();

    // When --select is used, filter physical nodes to only those whose target
    // has a backend available. Upstream models on other engines (e.g., Spark)
    // may be in the physical graph but shouldn't be executed.
    let selected_set: HashSet<&str> = execution_order.iter().map(|s| s.as_str()).collect();
    for phys_node in physical_graph.iter_in_order().filter(|n| {
        selected_set.contains(n.name.as_str())
            || n.logical_origins
                .iter()
                .any(|o| selected_set.contains(o.as_str()))
    }) {
        let model_name = &phys_node.name;
        let model = &phys_node.model_file;
        let backend = registry.get(&phys_node.target);
        let compiler = compilers.get(&phys_node.target);
        let schema = &registry.target_config(&phys_node.target).schema;
        let resolver = physical_graph.ephemeral_resolver(&phys_node.target);

        // Schema evolution check for incremental table models
        let mut force_full_refresh = false;
        if let PhysicalStrategy::Incremental { .. } = &phys_node.strategy {
            // Check if schema evolution is opted out via frontmatter
            let evolution_strategy = model
                .metadata
                .as_ref()
                .and_then(|m| m.schema_evolution.as_ref())
                .map(|se| &se.strategy);
            let use_alter = !matches!(
                evolution_strategy,
                Some(SchemaEvolutionStrategy::FullRefresh)
            );

            if use_alter {
                if let Ok(true) = backend.table_exists(schema, model_name).await {
                    let inferred_columns = infer_deployed_columns(&type_db, model);
                    if !inferred_columns.is_empty() {
                        let (column_defaults, backfill_exprs) =
                            migration::extract_evolution_maps(model.metadata.as_deref());

                        match migration::check_and_migrate(
                            backend,
                            &file_store,
                            model_name,
                            &model.content,
                            schema,
                            &inferred_columns,
                            args.allow_column_removal,
                            args.dry_run,
                            &column_defaults,
                            &backfill_exprs,
                        )
                        .await
                        {
                            Ok(migration::SchemaEvolutionResult::FirstDeployment) => {}
                            Ok(migration::SchemaEvolutionResult::NoChange) => {}
                            Ok(migration::SchemaEvolutionResult::Migrated { statements }) => {
                                info!("Schema evolved ({} ALTER statement(s)):", statements.len());
                                for stmt in &statements {
                                    info!("    {}", stmt);
                                }
                            }
                            Ok(migration::SchemaEvolutionResult::FullRefreshRequired {
                                reason,
                            }) => {
                                info!("Schema change requires full refresh: {}", reason);
                                force_full_refresh = true;
                            }
                            Ok(migration::SchemaEvolutionResult::ColumnRemovalBlocked {
                                columns,
                            }) => {
                                return Err(anyhow::anyhow!(
                                    "Schema evolution for '{}' would remove columns: {}. \
                                 Use --allow-column-removal to permit this.",
                                    model_name,
                                    columns.join(", ")
                                ));
                            }
                            Err(e) => {
                                warn!(
                                "Schema evolution check failed: {}. Continuing with incremental.",
                                e
                            );
                            }
                        }
                    }
                }
            }
        }

        // If schema evolution requires full refresh, override the strategy
        let effective_strategy = if force_full_refresh {
            &PhysicalStrategy::FullRefresh
        } else {
            &phys_node.strategy
        };

        let result = match effective_strategy {
            PhysicalStrategy::Incremental {
                config: inc_config,
                time_range: range,
                plan_steps: Some(steps),
            } => {
                let resolved_strategy = backend.resolve_strategy(inc_config);
                info!(
                    "Running model: {} (cube split + incremental/{})",
                    model_name,
                    strategy_label(&resolved_strategy),
                );

                // Compute temporal windows (lookback/lookahead from AST + data latency)
                let model_latency = model
                    .metadata
                    .as_ref()
                    .and_then(|m| m.columns.get(&inc_config.event_time_column))
                    .and_then(|c| c.data_latency.as_ref());
                let windows = compute_incremental_windows(
                    &model.content,
                    inc_config,
                    sources.as_ref(),
                    model_latency,
                    range,
                );

                if windows.effective_window.lookback_days > 0
                    || windows.effective_window.lookahead_days > 0
                {
                    debug!("Temporal window: {}", windows.effective_window.explanation);
                }

                let partition_values = generate_partition_values(
                    &windows.partition_range.start,
                    &windows.partition_range.end,
                    &inc_config.granularity,
                )?;
                let partition = PartitionSpec {
                    column: inc_config.partition_column.clone(),
                    values: partition_values,
                };

                executor::execute_plan_incremental(
                    backend,
                    model_name,
                    steps,
                    schema,
                    partition,
                    &inc_config.event_time_column,
                    &windows.filter_range,
                    resolved_strategy,
                    inc_config.unique_key.clone(),
                    args.show_results,
                )
                .await
                .with_context(|| format!("Failed to execute model: {}", model_name))?
            }
            PhysicalStrategy::CubeSplit { steps } => {
                // Cube split only (full refresh)
                info!("Running model: {} (cube split)", model_name);

                executor::execute_plan(backend, model_name, steps, schema, args.show_results)
                    .await
                    .with_context(|| format!("Failed to execute model: {}", model_name))?
            }
            PhysicalStrategy::Incremental {
                config: inc_config,
                time_range: range,
                plan_steps: None,
            } => {
                let resolved_strategy = backend.resolve_strategy(inc_config);

                // Compute temporal windows (lookback/lookahead from AST + data latency)
                let model_latency = model
                    .metadata
                    .as_ref()
                    .and_then(|m| m.columns.get(&inc_config.event_time_column))
                    .and_then(|c| c.data_latency.as_ref());
                let windows = compute_incremental_windows(
                    &model.content,
                    inc_config,
                    sources.as_ref(),
                    model_latency,
                    range,
                );

                // Compute smart batching based on batch safety analysis
                let (batch_safety, batches) = compute_batches_for_model(
                    &model.content,
                    inc_config,
                    range,
                    &windows.filter_range,
                    &backfill_options,
                )
                .with_context(|| format!("Failed to compute batches for model: {}", model_name))?;

                let batch_count = batches.len().max(1);
                let safety_label = match &batch_safety {
                    smelt_planner::BatchSafety::FullyBatchSafe => "batch-safe".to_string(),
                    smelt_planner::BatchSafety::BoundedSafe {
                        max_chunk_days,
                        context_days,
                        ..
                    } => format!(
                        "bounded {}d chunks/{}d context",
                        max_chunk_days, context_days
                    ),
                    smelt_planner::BatchSafety::PerPartitionOnly { .. } => {
                        "per-partition".to_string()
                    }
                };

                info!(
                    "Running model: {} (incremental/{}, {} batch(es), {})",
                    model_name,
                    strategy_label(&resolved_strategy),
                    batch_count,
                    safety_label,
                );

                if windows.effective_window.lookback_days > 0
                    || windows.effective_window.lookahead_days > 0
                {
                    debug!("Temporal window: {}", windows.effective_window.explanation);
                }

                // Use computed batches, or fall back to single batch for the full range
                let effective_batches: Vec<smelt_cli::BackfillBatch> = if batches.is_empty() {
                    vec![smelt_cli::BackfillBatch {
                        partition_range: windows.partition_range.clone(),
                        filter_range: windows.filter_range.clone(),
                    }]
                } else {
                    batches
                };

                let mut last_result = None;
                for (i, batch) in effective_batches.iter().enumerate() {
                    if effective_batches.len() > 1 {
                        debug!(
                            "Batch {}/{}: [{}, {})",
                            i + 1,
                            effective_batches.len(),
                            batch.partition_range.start,
                            batch.partition_range.end,
                        );
                    }

                    let clean_sql = smelt_parser::strip_frontmatter(&model.content);
                    let transformed_sql = inject_time_filter(
                        &clean_sql,
                        &inc_config.event_time_column,
                        &batch.filter_range,
                    )
                    .with_context(|| {
                        format!("Failed to transform SQL for model: {}", model_name)
                    })?;

                    let compiled = compiler
                        .compile_with_sql_and_ephemerals(model, schema, &transformed_sql, resolver)
                        .with_context(|| format!("Failed to compile model: {}", model_name))?;

                    if args.verbose {
                        debug!("Transformed SQL:\n{}", compiled.sql);
                    }

                    let partition_values = generate_partition_values(
                        &batch.partition_range.start,
                        &batch.partition_range.end,
                        &inc_config.granularity,
                    )?;

                    if effective_batches.len() == 1 {
                        debug!(
                            "Partitions to update: {} ({} {})",
                            if partition_values.len() <= 3 {
                                partition_values.join(", ")
                            } else {
                                format!(
                                    "{}, ..., {}",
                                    partition_values
                                        .first()
                                        .expect("partition_values is non-empty when len > 3"),
                                    partition_values
                                        .last()
                                        .expect("partition_values is non-empty when len > 3")
                                )
                            },
                            partition_values.len(),
                            granularity_label(&inc_config.granularity),
                        );
                    }

                    let partition = PartitionSpec {
                        column: inc_config.partition_column.clone(),
                        values: partition_values,
                    };

                    let result = executor::execute_model_incremental(
                        backend,
                        &compiled,
                        schema,
                        partition,
                        resolved_strategy.clone(),
                        inc_config.unique_key.clone(),
                        args.show_results,
                    )
                    .await
                    .with_context(|| format!("Failed to execute model: {}", model_name))?;

                    if effective_batches.len() > 1 {
                        debug!("batch {} rows ({:?})", result.row_count, result.duration);
                    }

                    last_result = Some(result);
                }

                last_result.expect("at least one batch must have been processed")
            }
            PhysicalStrategy::FullRefresh => {
                if time_range.is_some() {
                    info!(
                        "Running model: {} (full refresh - not configured for incremental)",
                        model_name
                    );
                } else {
                    info!("Running model: {}", model_name);
                }

                let compiled = compiler
                    .compile_with_ephemerals(model, schema, resolver)
                    .with_context(|| format!("Failed to compile model: {}", model_name))?;

                if args.verbose {
                    debug!("Compiled SQL:\n{}", compiled.sql);
                }

                executor::execute_model(backend, &compiled, schema, args.show_results)
                    .await
                    .with_context(|| format!("Failed to execute model: {}", model_name))?
            }
        };

        info!(
            "{} done ({} rows, {:?})",
            result.model_name, result.row_count, result.duration
        );

        if let Some(ref batches) = result.preview {
            println!("\n  Preview:");
            pretty::print_batches(batches).with_context(|| "Failed to print result preview")?;
            println!();
        }

        // Save deployed schema after successful execution (best-effort)
        if !args.dry_run {
            let inferred_columns = infer_deployed_columns(&type_db, model);
            if !inferred_columns.is_empty() {
                let existing_version = file_store
                    .load_schema(model_name)
                    .ok()
                    .flatten()
                    .map(|s| s.version);
                if let Err(e) = migration::save_deployed_schema(
                    &file_store,
                    model_name,
                    &model.content,
                    &inferred_columns,
                    existing_version,
                ) {
                    warn!("Failed to save deployed schema: {}", e);
                }
            }
        }

        results.push((result, phys_node.strategy.clone()));
    }

    // 9. Save run manifest and update intervals
    let run_id = generate_run_id();
    let mut manifest = RunManifest {
        run_id: run_id.clone(),
        started_at: run_start,
        completed_at: Some(Utc::now()),
        models: HashMap::new(),
    };

    let mut interval_store = file_store.load_intervals().unwrap_or_default();

    for (result, strategy) in &results {
        let (strategy_name, tr, partitions, batch_safety_label) = match strategy {
            PhysicalStrategy::Incremental {
                config: inc,
                time_range: range,
                ..
            } => {
                let phys_node = physical_graph.get_node(&result.model_name);
                let model_target = phys_node.map(|n| n.target.as_str()).unwrap_or(&args.target);
                let backend_strategy = registry.get(model_target).resolve_strategy(inc);
                (
                    strategy_label(&backend_strategy).to_string(),
                    Some(TimeRangeRecord {
                        start: range.start.clone(),
                        end: range.end.clone(),
                    }),
                    generate_partition_values(&range.start, &range.end, &inc.granularity)
                        .unwrap_or_default(),
                    Some("incremental".to_string()),
                )
            }
            PhysicalStrategy::CubeSplit { .. } => ("cube_split".to_string(), None, vec![], None),
            PhysicalStrategy::FullRefresh => ("full_refresh".to_string(), None, vec![], None),
        };

        manifest.models.insert(
            result.model_name.clone(),
            ModelRunRecord {
                strategy: strategy_name,
                time_range: tr.clone(),
                partitions_updated: partitions,
                row_count: result.row_count,
                duration_ms: result.duration.as_millis() as u64,
                batch_safety: batch_safety_label,
            },
        );

        // Update interval tracking for incremental models
        if let Some(ref range) = tr {
            let model = graph.get_model(&result.model_name)?;
            let model_hash = compute_model_hash(&model.content);
            let intervals = interval_store.get_or_create(&result.model_name, &model_hash);
            intervals.record_interval(&range.start, &range.end);
        }
    }

    // Save state (best-effort — don't fail the run if state can't be saved)
    if let Err(e) = file_store.save_run(&manifest) {
        warn!("Failed to save run manifest: {}", e);
    }
    if let Err(e) = file_store.save_intervals(&interval_store) {
        warn!("Failed to save interval store: {}", e);
    }

    // 10. Summary
    info!("{}", "=".repeat(60));
    info!("Summary (run: {})", run_id);
    info!("{}", "=".repeat(60));
    info!("Executed {} models successfully", results.len());

    let total_duration: std::time::Duration = results.iter().map(|(r, _)| r.duration).sum();
    info!("Total time: {:?}", total_duration);

    Ok(())
}
