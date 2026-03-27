use anyhow::{Context, Result};
use chrono::NaiveDate;
use smelt_backend::PartitionSpec;
use smelt_cli::{
    compute_backbuild_plans, discover_python_models, executor, find_project_root,
    format_plan_summary, inject_time_filter, parse_selector, BackendRegistry, BackfillOptions,
    CompilerRegistry, Config, LogicalGraph, Materialization, ModelDiscovery, PhysicalGraphBuilder,
    SourcesConfig, TimeRange,
};
use smelt_planner::Frontmatter;
use std::collections::{HashMap, HashSet};

use tracing::{debug, info};

use crate::helpers::{generate_partition_values, strategy_label};
use crate::BackbuildArgs;

pub async fn backbuild(args: BackbuildArgs) -> Result<()> {
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

    // 3. Discover models
    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Filter out test models — they shouldn't be materialized
    models.retain(|m| !m.is_test());

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
            "No models found in model paths: {}",
            config.model_paths.join(", ")
        ));
    }

    // 4. Build logical graph
    let graph = LogicalGraph::build(models, sources.as_ref(), &config, &args.target)
        .with_context(|| "Failed to build logical graph")?;

    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    // 5. Parse selector — backbuild always includes upstream (+model)
    let selector = parse_selector(&args.selector)
        .with_context(|| format!("Invalid selector: {}", args.selector))?;

    // Force upstream inclusion for backbuild
    let selectors = vec![smelt_cli::Selector {
        include_upstream: true,
        ..selector
    }];

    let selected = graph
        .select_models(&selectors)
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
        sources.as_ref(),
        &requested_range,
        &backfill_options,
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
    graph
        .validate_cross_backend_refs()
        .with_context(|| "Cross-backend reference validation failed")?;

    let needed_targets: HashSet<String> = execution_order
        .iter()
        .map(|name| {
            graph
                .get_node(name)
                .expect("execution_order only contains valid node names")
                .target
                .clone()
        })
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
    let compilers = CompilerRegistry::new(&config, &needed_target_configs);

    if let Some(ref source_config) = sources {
        executor::validate_sources(registry.get(&args.target), source_config)
            .await
            .with_context(|| "Source validation failed")?;
    }

    // Build physical graph for ephemeral resolver construction
    let target_schemas: HashMap<String, String> = needed_targets
        .iter()
        .map(|name| (name.clone(), registry.target_config(name).schema.clone()))
        .collect();
    let physical_graph = PhysicalGraphBuilder::new(
        &graph,
        &[],
        Some(requested_range.clone()),
        &compilers,
        target_schemas,
    )
    .build()
    .with_context(|| "Failed to build physical graph for backbuild")?;

    info!("{}", "=".repeat(60));
    info!("Executing backbuild...");
    info!("{}", "=".repeat(60));

    let mut total_results = Vec::new();

    for plan in &plans {
        // Skip ephemeral models (absorbed into physical graph's resolvers)
        let node = graph.get_node(&plan.model_name)?;
        if node.materialization == Materialization::Ephemeral {
            debug!("{} (ephemeral - inlined as CTE)", plan.model_name);
            continue;
        }

        let model = graph.get_model(&plan.model_name)?;
        let model_target = &node.target;
        let backend = registry.get(model_target);
        let compiler = compilers.get(model_target);
        let schema = &registry.target_config(model_target).schema;
        let resolver = physical_graph.ephemeral_resolver(model_target);

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
            .get_incremental_with_metadata(
                &plan.model_name,
                model.metadata.as_ref().map(|b| b.as_ref()),
            )
            .cloned()
            .or_else(|| frontmatter.as_ref().and_then(|f| f.incremental.clone()));

        let inc_config = match inc_config {
            Some(c) => c,
            None => continue,
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
                &inc_config.event_time_column,
                &batch.filter_range,
            )
            .with_context(|| format!("Failed to transform SQL for model: {}", plan.model_name))?;

            let compiled = compiler
                .compile_with_sql_and_ephemerals(model, schema, &transformed_sql, resolver)
                .with_context(|| format!("Failed to compile model: {}", plan.model_name))?;

            if args.verbose {
                debug!("Compiled SQL:\n{}", compiled.sql);
            }

            let partition_values = generate_partition_values(
                &batch.partition_range.start,
                &batch.partition_range.end,
                &inc_config.granularity,
            )?;

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
