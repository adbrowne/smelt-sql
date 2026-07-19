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

use super::run_setup::*;
use crate::RunArgs;

pub async fn run(args: RunArgs, scope: Option<&str>) -> Result<()> {
    let run_start = Utc::now();

    // 1. Resolve project root + config.
    let project_dir = smelt_cli::find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;
    info!("Project directory: {}", project_dir.display());
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
    validate_materialization_configs(&models, &config)?;

    // 4. Build dependency graph + resolve selectors.
    let mut graph = DependencyGraph::build(models.clone(), sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    graph.add_seeds(&seeds);
    graph.warn_unused_ephemerals(&config);

    // `--since-upstream`: forward propagation from caller-declared per-source
    // deltas (`incremental_models.md` §CLI). A separate codepath from the
    // regular selector-driven run below — it computes its own (model,
    // region) set from the propagation graph rather than a --select/--start/
    // --end window, then loops `execute_project` once per propagated region.
    if args.since_upstream {
        return run_since_upstream(args, &project_dir, &config, &models, graph).await;
    }

    let mut gen_salsa_db = smelt_cli::init_db(
        &project_dir,
        &discovery.discover_models().unwrap_or_default(),
    );
    gen_salsa_db.set_active_target(Some(Arc::from(args.target.as_str())));
    let gen_salsa_ws =
        smelt_db::Workspace::try_get(&gen_salsa_db).expect("workspace not initialized");
    let gen_salsa_project = gen_salsa_db
        .project_input(&project_dir)
        .expect("project not initialized");

    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
    let resolved_select = resolve_selector_args(
        &gen_salsa_db,
        gen_salsa_ws,
        gen_salsa_project,
        active_scope.as_ref(),
        &args.select,
    )
    .map_err(|e| anyhow::anyhow!("{}", e))?;
    let resolved_exclude = resolve_selector_args(
        &gen_salsa_db,
        gen_salsa_ws,
        gen_salsa_project,
        active_scope.as_ref(),
        &args.exclude,
    )
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Scope parse-error gating: when --select is active, only gate on models
    // in the selected subgraph + their transitive deps. An unrelated broken
    // model must not abort a scoped run.
    {
        let gate_names = parse_error_gate_set(&graph, &resolved_select, &config);
        match gate_names {
            Some(ref names) => {
                let scoped: Vec<smelt_cli::ModelFile> = models
                    .iter()
                    .filter(|m| names.contains(&m.canonical_path()))
                    .cloned()
                    .collect();
                check_parse_errors(&scoped)?;
            }
            None => check_parse_errors(&models)?,
        }
    }

    // Reject directly-selected ephemeral models.
    if !resolved_select.is_empty() {
        for s in &resolved_select {
            let sel = smelt_cli::parse_selector(s)
                .with_context(|| format!("Invalid selector '{}'", s))?;
            if let smelt_cli::SelectionMethod::ModelName(name) = &sel.method {
                if !sel.include_upstream && !sel.include_downstream {
                    if let Ok(model) = graph.get_model(name) {
                        let mat = config
                            .get_materialization_with_metadata(name, model.metadata.as_deref());
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
        compute_auto_time_range(&project_dir, &args.target, &graph)
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
    let salsa_db = build_execute_salsa_db(
        &discovery,
        &function_files,
        &models,
        &project_dir,
        &args.target,
    )?;

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
        run_checks: false,
        checks: vec![],
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
        if !args.select.is_empty() && planned == 0 {
            eprintln!("smelt: no models matched the selector(s)");
        }
        info!(
            "[DRY RUN] Compiled {} model(s); no execution performed.",
            planned
        );
        return Ok(());
    }

    info!("{}", "=".repeat(60));
    info!("Summary (run: {})", run_id);
    info!("{}", "=".repeat(60));
    info!("Executed {} models successfully", outcome.models.len());
    let _ = (run_start, std::time::Instant::now().elapsed());
    Ok(())
}

/// `smelt run --since-upstream` — forward propagation from caller-declared
/// per-source deltas (`incremental_models.md` §CLI, §"The graph layer").
///
/// Argument parsing + reporter wiring only (per the Run Pipeline Parity
/// invariant): `smelt_runtime::propagation` computes the real per-workspace
/// propagation graph and the exact `(model, region)` set to run; this
/// function only pairs the `--source`/`--landed` flags, prints the dirty set
/// the plan computed, and loops the SAME `execute_project` entry point every
/// other run path uses, once per propagated region.
async fn run_since_upstream(
    args: RunArgs,
    project_dir: &std::path::Path,
    config: &Config,
    models: &[smelt_cli::ModelFile],
    graph: DependencyGraph,
) -> Result<()> {
    let deltas = smelt_runtime::propagation::pair_source_deltas(
        &args.since_upstream_source,
        &args.since_upstream_landed,
    )
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Each `--source` address must resolve to either a declared source or an
    // upstream maintained model (`incremental_models.md` §"Upstream model
    // edges": "`--source <address>` accepts either a declared source or an
    // upstream maintained model"). Resolution goes through the canonical
    // `resolve_ref_path` resolver — no parallel leaf-only path (`cli.md`
    // §"Argument resolution"). An address that is neither is a named error,
    // never a silent no-op.
    {
        let db = smelt_cli::init_db(project_dir, models);
        let ws = smelt_db::Workspace::try_get(&db)
            .ok_or_else(|| anyhow::anyhow!("workspace not initialized for --source resolution"))?;
        for addr in &args.since_upstream_source {
            let stripped = addr.strip_prefix("smelt.").unwrap_or(addr);
            let segs: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
            match smelt_db::resolve_ref_path(&db, ws, segs) {
                Some(r)
                    if matches!(r.kind, smelt_db::RefKind::Source | smelt_db::RefKind::Model) => {}
                Some(r) => {
                    return Err(anyhow::anyhow!(
                        "--source '{addr}' resolves to a {:?}, not a declared source or a \
                         maintained model — forward propagation seeds a delta only on a source \
                         or an upstream maintained model",
                        r.kind
                    ));
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "--source '{addr}' is neither a declared source nor a maintained model in \
                         this project"
                    ));
                }
            }
        }
    }

    let source_infos = smelt_core::discover_source_infos(project_dir, &config.paths);
    let order = graph
        .execution_order()
        .with_context(|| "Failed to compute execution order")?;

    let plan =
        smelt_runtime::propagation::plan_since_upstream(models, &source_infos, &order, &deltas)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

    print!("{}", plan.dirty_set_report);
    if plan.runs.is_empty() {
        eprintln!("smelt: --since-upstream propagated nothing — no model has dirt to run");
        return Ok(());
    }

    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let (salsa_models, function_files) =
        discover_models_for_run(&discovery, &args.target, project_dir, config)?;
    let seeds = smelt_core::discover_seed_infos_with_sidecars(project_dir, &config.paths);
    let ephemeral_seed_ctes = build_ephemeral_seed_ctes(&seeds);

    let salsa_db = build_execute_salsa_db(
        &discovery,
        &function_files,
        &salsa_models,
        project_dir,
        &args.target,
    )?;

    let config_arc = Arc::new(config.clone());
    let graph_arc = Arc::new(tokio::sync::Mutex::new(graph));
    let db_arc = Arc::new(tokio::sync::Mutex::new(salsa_db));
    let reporter = CliReporter::new(args.verbose, args.dry_run, args.show_results);
    let backend_factory = CliBackendFactory {
        database_override: args.database.clone(),
    };

    for run in &plan.runs {
        info!(
            "[--since-upstream] running {} over {}",
            run.model,
            match (&run.start, &run.end) {
                (Some(s), Some(e)) => format!("[{s}, {e})"),
                _ => "whole table".to_string(),
            }
        );
        let request = ExecuteRequest {
            target: args.target.clone(),
            select: vec![run.model.clone()],
            exclude: vec![],
            start: run.start.clone(),
            end: run.end.clone(),
            batch_size_days: args.batch_size,
            per_partition: args.per_partition,
            full_refresh: false,
            dry_run: args.dry_run,
            enforce_safety: !args.allow_downgrade,
            allow_column_removal: args.allow_column_removal,
            allow_full_refresh: args.allow_full_refresh,
            ephemeral_seed_ctes: ephemeral_seed_ctes.clone(),
            run_checks: false,
            checks: Vec::new(),
        };
        let run_id = generate_run_id();
        smelt_runtime::execute_project(
            run_id,
            request,
            Arc::clone(&config_arc),
            Arc::clone(&graph_arc),
            Arc::clone(&db_arc),
            project_dir,
            &backend_factory,
            &reporter,
            CancellationToken::new(),
        )
        .await
        .with_context(|| format!("--since-upstream run of '{}' failed", run.model))?;
    }

    Ok(())
}
