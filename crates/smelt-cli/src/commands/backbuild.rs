use anyhow::{Context, Result};
use chrono::NaiveDate;
use smelt_backend::PartitionRange;
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_argument},
    compiler::UpstreamSchemas,
    compute_backbuild_plans, discover_python_models, executor, find_project_root,
    format_plan_summary, init_db, inject_time_filter, parse_selector, BackendRegistry,
    BackfillOptions, CompilerRegistry, Config, Materialization, ModelDiscovery, SourcesConfig,
    TimeRange,
};
use smelt_core::graph::DependencyGraph;
use smelt_planner::Frontmatter;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tracing::{debug, info};

use crate::helpers::strategy_label;
use crate::BackbuildArgs;

pub async fn backbuild(args: BackbuildArgs, scope: Option<&str>) -> Result<()> {
    // 1. Find project root
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    info!("Project directory: {}", project_dir.display());

    // 2. Load configuration
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    info!("Project: {} (version {})", config.name, config.version);

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

    let sources = SourcesConfig::load(&project_dir).ok();

    // Seeds are valid `smelt.ref()` targets too (bug #2 in 20260417 follow-up).
    let seeds = smelt_core::discover_seed_infos(&project_dir, &config.paths);

    // 3. Discover models
    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Filter out test models — they shouldn't be materialized
    models.retain(|m| !m.is_test());

    // Function files (smelt.define / smelt.extern) must be registered with
    // the Salsa DB so `build_fn_body_map` can extract bodies for
    // `smelt.fn.*` expansion. They are NOT materialisable models so they
    // stay out of `models` and the dependency graph.
    let function_files = discovery
        .discover_function_files()
        .with_context(|| "Failed to discover function files")?;

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
        models.extend(python_models);
    }

    if models.is_empty() {
        return Err(anyhow::anyhow!(
            "No models found in paths: {}",
            config.paths.join(", ")
        ));
    }

    // Initialise the Salsa DB early so scope resolution can delegate to
    // smelt_db::resolve_ref_path / leaf_did_you_mean.
    let scope_db = init_db(&project_dir, &models);
    let scope_ws = smelt_db::Workspace::try_get(&scope_db).expect("workspace not initialized");
    let scope_project = scope_db
        .project_input(&project_dir)
        .expect("project not initialized");

    // 4. Build dependency graph
    let mut graph = DependencyGraph::build(models, sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    graph.add_seeds(&seeds);

    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    // 5. Parse selector — backbuild always includes upstream (+model)
    // Resolve the selector through scope before parsing.
    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
    let resolved_selector = resolve_argument(
        &scope_db,
        scope_ws,
        scope_project,
        active_scope.as_ref(),
        &args.selector,
    )
    .unwrap_or_else(|_| args.selector.clone()); // fall back for tag/special selectors
    let selector = parse_selector(&resolved_selector)
        .with_context(|| format!("Invalid selector: {}", resolved_selector))?;

    // Force upstream inclusion for backbuild
    let selectors = vec![smelt_cli::Selector {
        include_upstream: true,
        ..selector
    }];

    let selected = graph
        .select_models(&selectors, &config)
        .with_context(|| "Failed to select models")?;

    if selected.is_empty() {
        info!("No models matched the selector");
        return Ok(());
    }

    let execution_order = graph
        .filtered_execution_order(&selected)
        .with_context(|| "Failed to determine execution order")?;

    info!(
        "Backbuild execution order: {}",
        execution_order
            .iter()
            .enumerate()
            .map(|(i, name)| format!("{}. {}", i + 1, name))
            .collect::<Vec<_>>()
            .join(" -> ")
    );

    // 6. Validate time range
    let requested_range = TimeRange {
        start: args.start.clone(),
        end: args.end.clone(),
    };

    NaiveDate::parse_from_str(&args.start, "%Y-%m-%d")
        .with_context(|| format!("Invalid start date: {}", args.start))?;
    NaiveDate::parse_from_str(&args.end, "%Y-%m-%d")
        .with_context(|| format!("Invalid end date: {}", args.end))?;

    info!("Target range: {} to {} (exclusive)", args.start, args.end);

    // 7. Compute DAG-aware backfill plans
    let target_model = selectors[0]
        .method
        .model_name()
        .ok_or_else(|| anyhow::anyhow!("Backbuild selector must specify a model name"))?;

    let backfill_options = BackfillOptions {
        batch_size_days: args.batch_size,
        per_partition: args.per_partition,
    };

    let plans = compute_backbuild_plans(
        target_model,
        &execution_order,
        &graph,
        &config,
        sources.as_ref(),
        &requested_range,
        &backfill_options,
        args.allow_downgrade,
    )
    .with_context(|| "Failed to compute backbuild plans")?;

    // 8. Display plan
    info!("Backfill plan:");
    info!("{}", format_plan_summary(&plans));

    if args.dry_run {
        info!("[DRY RUN] Skipping execution");
        return Ok(());
    }

    // 9. Compute needed targets and create backends
    // Compute target assignments from config for each model in execution order.
    let target_assignments: HashMap<String, String> = execution_order
        .iter()
        .map(|name| {
            let model = graph
                .get_model(name)
                .expect("execution_order only contains valid model names");
            let target = config.get_target(name, model.metadata.as_deref(), &args.target);
            (name.clone(), target)
        })
        .collect();

    let cross_edges_raw = graph.find_cross_backend_edges(&target_assignments);
    // Convert raw tuples to named structs for logging.
    let cross_edges: Vec<(String, String, String, String)> = cross_edges_raw;
    if !cross_edges.is_empty() {
        info!(
            "Cross-engine references detected ({} transfer(s) via Parquet):",
            cross_edges.len()
        );
        for (downstream, upstream, downstream_target, upstream_target) in &cross_edges {
            info!(
                "  {} ({}) -> {} ({})",
                upstream, upstream_target, downstream, downstream_target
            );
        }
    }

    let needed_targets: HashSet<String> = target_assignments.values().cloned().collect();
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
    // Paths are computed from target config (warehouse field) so we don't need
    // the upstream backend to be instantiated.
    if !cross_edges.is_empty() {
        let mut refs_by_target: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (_downstream, upstream, downstream_target, upstream_target) in &cross_edges {
            let upstream_target_config = &config.targets[upstream_target];
            let upstream_schema = &upstream_target_config.schema;
            let rel_path = if let Some(ref warehouse) = upstream_target_config.warehouse {
                Some(std::path::PathBuf::from(format!(
                    "{}/{}/{}",
                    warehouse, upstream_schema, upstream
                )))
            } else if needed_targets.contains(upstream_target) {
                registry
                    .get(upstream_target)
                    .materialized_path(upstream_schema, upstream)
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
                    .entry(downstream_target.clone())
                    .or_default()
                    .insert(upstream.clone(), parquet_expr);
            }
        }
        for (target, refs) in refs_by_target {
            compilers.set_cross_engine_refs(&target, refs);
        }
    }

    if let Some(ref source_config) = sources {
        executor::validate_sources(registry.get(&args.target), source_config)
            .await
            .with_context(|| "Source validation failed")?;
    }

    // Build all non-ephemeral models list for UpstreamSchemas construction.
    let all_models: Vec<smelt_core::ModelFile> =
        graph.iter_models().map(|(_, m)| m.clone()).collect();

    // Build a populated TypeContext for `apply_type_casts`.
    {
        let mut db_files: Vec<_> = all_models.clone();
        db_files.extend(function_files.iter().cloned());
        let type_db = init_db(&project_dir, &db_files);
        let upstream_schemas = Arc::new(UpstreamSchemas::from_database(
            &type_db,
            &project_dir,
            &all_models,
        )?);
        compilers.set_upstream_schemas_all(upstream_schemas);

        let workspace =
            smelt_db::Workspace::try_get(&type_db).expect("workspace not initialised by init_db");
        let fn_bodies = smelt_runtime::build_fn_body_map(&type_db, workspace);
        if !fn_bodies.is_empty() {
            compilers.set_function_bodies_all(fn_bodies);
        }
    }

    // Build ephemeral resolvers per target by collecting ephemeral models.
    let mut ephemeral_resolvers: HashMap<String, smelt_runtime::EphemeralResolver> = HashMap::new();
    for target_name in &needed_targets {
        let schema = &config.targets[target_name].schema;
        let ephemeral_models: Vec<(String, String)> = graph
            .iter_models()
            .filter(|(model_name, model_file)| {
                let metadata = model_file.metadata.as_deref();
                let mat = config.get_materialization_with_metadata(model_name, metadata);
                mat == smelt_core::config::Materialization::Ephemeral
                    && config.get_target(model_name, metadata, &args.target) == *target_name
            })
            .map(|(model_name, model_file)| (model_name.to_string(), model_file.content.clone()))
            .collect();
        let compiler = compilers.get(target_name);
        let resolver = compiler.build_ephemeral_resolver(&ephemeral_models, schema);
        ephemeral_resolvers.insert(target_name.clone(), resolver);
    }

    info!("{}", "=".repeat(60));
    info!("Executing backbuild...");
    info!("{}", "=".repeat(60));

    let mut total_results = Vec::new();

    // Static empty resolver fallback.
    static EMPTY_RESOLVER: std::sync::OnceLock<smelt_runtime::EphemeralResolver> =
        std::sync::OnceLock::new();

    for plan in &plans {
        let model = graph.get_model(&plan.model_name)?;
        let metadata = model.metadata.as_deref();

        // Skip ephemeral models (absorbed into resolvers)
        let mat = config.get_materialization_with_metadata(&plan.model_name, metadata);
        if mat == Materialization::Ephemeral {
            debug!("{} (ephemeral - inlined as CTE)", plan.model_name);
            continue;
        }

        let model_target = config.get_target(&plan.model_name, metadata, &args.target);
        let backend = registry.get(&model_target);
        let compiler = compilers.get(&model_target);
        let schema = &registry.target_config(&model_target).schema;
        let resolver = ephemeral_resolvers
            .get(&model_target)
            .unwrap_or_else(|| EMPTY_RESOLVER.get_or_init(smelt_runtime::EphemeralResolver::empty));

        if !plan.is_incremental {
            info!("{} (full refresh)", plan.model_name);
            let compiled = compiler
                .compile_with_ephemerals(model, schema, resolver)
                .with_context(|| format!("Failed to compile model: {}", plan.model_name))?;

            let result = executor::execute_model(backend, &compiled, schema, args.show_results)
                .await
                .with_context(|| format!("Failed to execute model: {}", plan.model_name))?;

            info!(
                "{} done ({} rows, {:?})",
                result.model_name, result.row_count, result.duration
            );
            total_results.push(result);
            continue;
        }

        let frontmatter = Frontmatter::parse(&model.content);
        let inc_config = config
            .get_incremental_with_metadata(&plan.model_name, metadata)
            .cloned()
            .or_else(|| frontmatter.as_ref().and_then(|f| f.incremental.clone()));

        let ts_config = config
            .get_timeseries_with_metadata(&plan.model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));

        let (inc_config, ts_config) = match (inc_config, ts_config) {
            (Some(i), Some(t)) => (i, t),
            _ => continue,
        };

        let resolved_strategy = backend.resolve_strategy(&inc_config);

        if plan.batches.len() == 1 {
            info!(
                "{} (incremental/{}, 1 batch)",
                plan.model_name,
                strategy_label(&resolved_strategy),
            );
        } else {
            info!(
                "{} (incremental/{}, {} batches)",
                plan.model_name,
                strategy_label(&resolved_strategy),
                plan.batches.len(),
            );
        }

        for (i, batch) in plan.batches.iter().enumerate() {
            if plan.batches.len() > 1 {
                debug!(
                    "Batch {}/{}: [{}, {})",
                    i + 1,
                    plan.batches.len(),
                    batch.partition_range.start,
                    batch.partition_range.end
                );
            }

            let clean_sql = smelt_parser::strip_frontmatter(&model.content);
            let transformed_sql = inject_time_filter(
                &clean_sql,
                &ts_config.event_time_column,
                &batch.filter_range,
            )
            .with_context(|| format!("Failed to transform SQL for model: {}", plan.model_name))?;

            let compiled = compiler
                .compile_with_sql_and_ephemerals(model, schema, &transformed_sql, resolver)
                .with_context(|| format!("Failed to compile model: {}", plan.model_name))?;

            if args.verbose {
                // Use println! (not tracing::debug!) so the SQL surfaces
                // without requiring RUST_LOG=debug. See the matching block
                // in `run.rs` and `docs/specs/cli.md` §"`--verbose`".
                println!("-- {}", plan.model_name);
                println!("{}", compiled.sql);
            }

            let partition = PartitionRange {
                column: ts_config.partition_column.clone(),
                start: batch.partition_range.start.clone(),
                end: batch.partition_range.end.clone(),
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
            .with_context(|| format!("Failed to execute model: {}", plan.model_name))?;

            info!(
                "{} done ({} rows, {:?})",
                result.model_name, result.row_count, result.duration
            );
            total_results.push(result);
        }
    }

    // Summary
    info!("{}", "=".repeat(60));
    info!("Backbuild Summary");
    info!("{}", "=".repeat(60));
    info!("Executed {} step(s) successfully", total_results.len());

    let total_duration: std::time::Duration = total_results.iter().map(|r| r.duration).sum();
    info!("Total time: {:?}", total_duration);

    Ok(())
}
