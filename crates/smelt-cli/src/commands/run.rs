use anyhow::{Context, Result};
use chrono::Utc;
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_selector_args},
    backend_factory::CliBackendFactory,
    reporter::{format_strategy, CliReporter},
    Config, ModelDiscovery, SourcesConfig,
};
use smelt_core::graph::DependencyGraph;
use smelt_runtime::types::ExecuteRequest;
use smelt_state::generate_run_id;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use tracing::info;

use crate::RunArgs;
use super::run_setup::*;

pub async fn run(args: RunArgs, scope: Option<&str>) -> Result<()> {
    let run_start = Utc::now();

    // 1. Resolve project root + config.
    let project_dir = smelt_cli::find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;
    info!("Project directory: {}", project_dir.display());
    let config = Config::load(&project_dir)
        .with_context(|| "Failed to load smelt.yml configuration")?;
    info!("Project: {} (version {})", config.name, config.version);
    if !config.targets.contains_key(&args.target) {
        return Err(anyhow::anyhow!(
            "Target '{}' not found in smelt.yml. Available targets: {}",
            args.target,
            config.targets.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }

    // 2. Load optional sources + seeds.
    let sources = SourcesConfig::load(&project_dir).ok();
    if let Some(ref sc) = sources {
        let n: usize = sc.sources.iter().map(|s| s.tables.len()).sum();
        info!("Loaded {} source tables", n);
    }
    let seeds = smelt_core::discover_seed_infos_with_sidecars(&project_dir, &config.paths);
    if !seeds.is_empty() {
        info!("Discovered {} seed(s) as ref targets", seeds.len());
    }

    // 3. Discover models and function files.
    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let (models, function_files) =
        discover_models_for_run(&discovery, &args.target, &project_dir, &config)?;
    let ephemeral_seed_ctes = build_ephemeral_seed_ctes(&seeds);

    if models.is_empty() {
        return Err(anyhow::anyhow!(
            "No models found in paths: {}",
            config.paths.join(", ")
        ));
    }
    info!("Found {} models total", models.len());
    check_parse_errors(&models)?;
    validate_materialization_configs(&models, &config)?;

    // 4. Build dependency graph + resolve selectors.
    let mut graph = DependencyGraph::build(models.clone(), sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    graph.add_seeds(&seeds);
    graph.warn_unused_ephemerals(&config);

    let mut gen_salsa_db =
        smelt_cli::init_db(&project_dir, &discovery.discover_models().unwrap_or_default());
    gen_salsa_db.set_active_target(Some(Arc::from(args.target.as_str())));
    let gen_salsa_ws =
        smelt_db::Workspace::try_get(&gen_salsa_db).expect("workspace not initialized");
    let gen_salsa_project = gen_salsa_db
        .project_input(&project_dir)
        .expect("project not initialized");

    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
    let resolved_select = resolve_selector_args(
        &gen_salsa_db, gen_salsa_ws, gen_salsa_project, active_scope.as_ref(), &args.select,
    )
    .map_err(|e| anyhow::anyhow!("{}", e))?;
    let resolved_exclude = resolve_selector_args(
        &gen_salsa_db, gen_salsa_ws, gen_salsa_project, active_scope.as_ref(), &args.exclude,
    )
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Reject directly-selected ephemeral models.
    if !resolved_select.is_empty() {
        for s in &resolved_select {
            let sel = smelt_cli::parse_selector(s)
                .with_context(|| format!("Invalid selector '{}'", s))?;
            if let smelt_cli::SelectionMethod::ModelName(name) = &sel.method {
                if !sel.include_upstream && !sel.include_downstream {
                    if let Ok(model) = graph.get_model(name) {
                        let mat = config.get_materialization_with_metadata(
                            name,
                            model.metadata.as_deref(),
                        );
                        if mat == smelt_core::config::Materialization::Ephemeral {
                            return Err(anyhow::anyhow!(
                                "Cannot run ephemeral model '{}' directly — ephemeral models \
                                 are inlined as CTEs into downstream models.",
                                name
                            ));
                        }
                    }
                }
            }
        }
    }

    // 5. Compute time range.
    let effective_start = args.start.as_ref().or(args.event_time_start.as_ref());
    let effective_end = args.end.as_ref().or(args.event_time_end.as_ref());
    if let (Some(s), Some(e)) = (effective_start, effective_end) {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .with_context(|| format!("Invalid start date format: {}. Expected YYYY-MM-DD", s))?;
        chrono::NaiveDate::parse_from_str(e, "%Y-%m-%d")
            .with_context(|| format!("Invalid end date format: {}. Expected YYYY-MM-DD", e))?;
        info!("Time range: {} to {} (exclusive)", s, e);
    } else if effective_start.is_some() != effective_end.is_some() {
        return Err(anyhow::anyhow!(
            "Both --start and --end (or --event-time-start and --event-time-end) must be provided together"
        ));
    }
    let (auto_start, auto_end) = if args.auto && effective_start.is_none() {
        compute_auto_time_range(&project_dir, &graph)
            .map_or((None, None), |(s, e)| (Some(s), Some(e)))
    } else {
        (None, None)
    };
    let start_val = effective_start.cloned().or(auto_start);
    let end_val = effective_end.cloned().or(auto_end);

    // 6. Validate sources (best-effort, non-fatal).
    validate_sources_optional(
        sources.as_ref(),
        &config,
        &args.target,
        args.database.clone(),
        &resolved_select,
        &resolved_exclude,
        &project_dir,
    )
    .await;

    // 7. Build Salsa DB for execute_project, then run.
    let salsa_db =
        build_execute_salsa_db(&discovery, &function_files, &models, &project_dir, &args.target)?;

    let request = ExecuteRequest {
        target: args.target.clone(),
        select: resolved_select,
        exclude: resolved_exclude,
        start: start_val,
        end: end_val,
        batch_size_days: args.batch_size,
        per_partition: args.per_partition,
        full_refresh: false,
        dry_run: args.dry_run,
        enforce_safety: !args.allow_downgrade,
        allow_column_removal: args.allow_column_removal,
        allow_full_refresh: args.allow_full_refresh,
        ephemeral_seed_ctes,
    };

    let run_id = generate_run_id();
    let config_arc = Arc::new(config);
    let graph_arc = Arc::new(tokio::sync::Mutex::new(graph));
    let db_arc = Arc::new(tokio::sync::Mutex::new(salsa_db));
    let reporter = CliReporter::new(args.verbose, args.dry_run, args.show_results);
    let backend_factory = CliBackendFactory {
        database_override: args.database,
    };

    let outcome = smelt_runtime::execute_project(
        run_id.clone(),
        request,
        config_arc,
        graph_arc,
        db_arc,
        &project_dir,
        &backend_factory,
        &reporter,
        CancellationToken::new(),
    )
    .await?;

    // --show-plan works in both dry-run and live-run modes.
    if args.show_plan {
        if let Some(ref plan) = outcome.plan_summary {
            println!("Execution plan:");
            for record in &plan.models {
                println!("  {} [{}]", record.name, format_strategy(&record.strategy));
            }
        }
    }

    if args.dry_run {
        let planned = outcome
            .plan_summary
            .as_ref()
            .map(|ps| {
                ps.models
                    .iter()
                    .filter(|r| {
                        !matches!(
                            r.strategy,
                            smelt_runtime::types::ModelStrategy::Ephemeral
                                | smelt_runtime::types::ModelStrategy::Skipped { .. }
                        )
                    })
                    .count()
            })
            .unwrap_or(0);
        info!("[DRY RUN] Compiled {} model(s); no execution performed.", planned);
        return Ok(());
    }

    info!("{}", "=".repeat(60));
    info!("Summary (run: {})", run_id);
    info!("{}", "=".repeat(60));
    info!("Executed {} models successfully", outcome.models.len());
    let _ = (run_start, std::time::Instant::now().elapsed());
    Ok(())
}
