use anyhow::{anyhow, Context, Result};
use smelt_cli::argument_resolution::{compute_scope, resolve_argument};
use smelt_cli::{Config, ModelDiscovery};
use smelt_core::graph::DependencyGraph;
use smelt_runtime::types::ExecuteRequest;
use smelt_state::generate_run_id;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::run_setup::*;
use crate::{BuildArgs, RunArgs, SeedArgs};

pub async fn build(args: BuildArgs, scope: Option<&str>) -> Result<()> {
    if args.show_plan {
        return show_plan(args);
    }

    // Step 1: Seed
    let seed_args = SeedArgs {
        project_dir: args.project_dir.clone(),
        database: args.database.clone(),
        target: args.target.clone(),
        show_results: false,
        select: Vec::new(),
    };
    super::seed::run_seed(seed_args, scope).await?;

    if args.include_upstreams {
        return build_include_upstreams(args, scope).await;
    }

    // Step 2: Run
    let run_args = RunArgs {
        project_dir: args.project_dir,
        database: args.database,
        target: args.target,
        show_results: args.show_results,
        verbose: args.verbose,
        dry_run: false,
        event_time_start: args.event_time_start,
        event_time_end: args.event_time_end,
        select: args.select,
        exclude: args.exclude,
        start: None,
        end: None,
        batch_size: None,
        per_partition: false,
        auto: false,
        allow_column_removal: false,
        allow_full_refresh: false,
        allow_downgrade: args.allow_downgrade,
        show_plan: false,
        since_upstream: false,
        since_upstream_source: Vec::new(),
        since_upstream_landed: Vec::new(),
    };
    super::run::run(run_args, scope).await
}

/// `smelt build <model> --period <start>..<end> --include-upstreams` —
/// backward resolution (`maintenance_plan.md` §CLI, §"Backward resolution —
/// what must exist"). Argument parsing + reporter wiring only (per the Run
/// Pipeline Parity invariant): `smelt_runtime::propagation::
/// resolve_build_plan` computes the real per-workspace required-slices plan
/// over the SAME graph `--since-upstream` assembles; this function resolves
/// the target model name, prints the plan the resolution computed, and
/// loops the SAME `execute_project` entry point every other run/build path
/// uses, once per resolved ancestor region, ancestor-first, target last.
async fn build_include_upstreams(args: BuildArgs, scope: Option<&str>) -> Result<()> {
    let target_arg = args
        .file
        .clone()
        .ok_or_else(|| {
            anyhow!(
                "--include-upstreams requires the target model as a positional argument, e.g. \
                 `smelt build <model> --period <start>..<end> --include-upstreams`"
            )
        })?
        .to_string_lossy()
        .into_owned();
    let period_arg = args
        .period
        .clone()
        .ok_or_else(|| anyhow!("--include-upstreams requires --period <start>..<end>"))?;

    let project_dir = smelt_cli::find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let (models, function_files) =
        discover_models_for_run(&discovery, &args.target, &project_dir, &config)?;
    if models.is_empty() {
        return Err(anyhow!(
            "No models found in paths: {}",
            config.paths.join(", ")
        ));
    }
    check_parse_errors(&models)?;

    // Resolve the target positional argument (a model name/selector, e.g.
    // `gold`, `smelt.marts.gold`) to its canonical address the same way
    // `smelt explain <model>` does.
    let resolve_db = smelt_cli::init_db(&project_dir, &models);
    let resolve_ws = smelt_db::Workspace::try_get(&resolve_db).expect("workspace not initialized");
    let resolve_project = resolve_db
        .project_input(&project_dir)
        .expect("project not initialized");
    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
    let target = resolve_argument(
        &resolve_db,
        resolve_ws,
        resolve_project,
        active_scope.as_ref(),
        &target_arg,
    )
    .map_err(|e| anyhow!("{}", e))?;

    let period = smelt_runtime::propagation::parse_landed_range(&period_arg)
        .map_err(|e| anyhow!("{}", e))?;

    let source_infos = smelt_core::discover_source_infos(&project_dir, &config.paths);
    let plan =
        smelt_runtime::propagation::resolve_build_plan(&models, &source_infos, &target, period)
            .map_err(|e| anyhow!("{}", e))?;

    print!("{}", plan.report);
    if plan.build_order.is_empty() {
        eprintln!(
            "smelt: --include-upstreams resolved nothing to build for '{}' over {}",
            target, period_arg
        );
        return Ok(());
    }

    let seeds = smelt_core::discover_seed_infos_with_sidecars(&project_dir, &config.paths);
    let ephemeral_seed_ctes = build_ephemeral_seed_ctes(&seeds);
    let salsa_db = build_execute_salsa_db(
        &discovery,
        &function_files,
        &models,
        &project_dir,
        &args.target,
    )?;

    let mut graph = DependencyGraph::build(models.clone(), None)
        .with_context(|| "Failed to build dependency graph")?;
    graph.add_seeds(&seeds);

    let config_arc = Arc::new(config);
    let graph_arc = Arc::new(tokio::sync::Mutex::new(graph));
    let db_arc = Arc::new(tokio::sync::Mutex::new(salsa_db));
    let reporter = smelt_cli::reporter::CliReporter::new(args.verbose, false, args.show_results);
    let backend_factory = smelt_cli::backend_factory::CliBackendFactory {
        database_override: args.database.clone(),
    };

    for run in &plan.build_order {
        info!(
            "[--include-upstreams] building {} over {}",
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
            batch_size_days: None,
            per_partition: false,
            full_refresh: false,
            dry_run: false,
            enforce_safety: !args.allow_downgrade,
            allow_column_removal: false,
            allow_full_refresh: false,
            ephemeral_seed_ctes: ephemeral_seed_ctes.clone(),
        };
        let run_id = generate_run_id();
        smelt_runtime::execute_project(
            run_id,
            request,
            Arc::clone(&config_arc),
            Arc::clone(&graph_arc),
            Arc::clone(&db_arc),
            &project_dir,
            &backend_factory,
            &reporter,
            CancellationToken::new(),
        )
        .await
        .with_context(|| format!("--include-upstreams build of '{}' failed", run.model))?;
    }

    Ok(())
}

fn show_plan(args: BuildArgs) -> Result<()> {
    use std::path::Path;

    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_core::find_project_root_by_walking_up;
    use smelt_db::Workspace;
    use smelt_planner::logical_plan_rules::{apply_rules_to_fixed_point, show_plan_rules};
    use smelt_planner::plan_printer::format_plan;

    let file = args.file.ok_or_else(|| {
        anyhow!("--show-plan requires a model file path as a positional argument")
    })?;

    let abs_file = std::fs::canonicalize(&file)
        .with_context(|| format!("Failed to resolve model file path: {}", file.display()))?;

    // Prefer an explicit --project-dir if the user passed one (i.e. anything
    // other than the default "."); otherwise walk up from the file.
    let project_dir = if args.project_dir != Path::new(".") {
        args.project_dir.clone()
    } else {
        find_project_root_by_walking_up(&abs_file).ok_or_else(|| {
            anyhow!(
                "Could not locate smelt.yml above {}; pass --project-dir explicitly",
                abs_file.display()
            )
        })?
    };

    let config = Config::load(&project_dir).with_context(|| "Failed to load smelt.yml")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;
    let function_files = discovery
        .discover_function_files()
        .with_context(|| "Failed to discover function files")?;
    models.extend(function_files);

    let db = init_db(&project_dir, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let source_file = db.source_file(&abs_file).ok_or_else(|| {
        anyhow!(
            "File {} is not registered in the workspace",
            abs_file.display()
        )
    })?;

    // Path-prefix enforcement: same rule as the build path. Reject
    // stem-included call paths before printing the plan.
    {
        let diags = smelt_db::file_diagnostics(&db, ws, source_file);
        let mut fn_path_errors: Vec<String> = Vec::new();
        for diag in diags {
            if diag.code == Some(smelt_db::DiagnosticCode::UnknownSmeltFn) {
                fn_path_errors.push(diag.message.clone());
            }
        }
        if !fn_path_errors.is_empty() {
            for err in &fn_path_errors {
                eprintln!("error: {err}");
            }
            return Err(anyhow!(
                "Unknown smelt function call(s) — the filename stem is not a path component; \
                 see `smelt docs show concepts/functions`.\n{}",
                fn_path_errors.join("\n")
            ));
        }
    }

    let plan = smelt_db::logical_plan(&db, ws, source_file)
        .ok_or_else(|| anyhow!("File {} did not parse as a valid model", abs_file.display()))?;

    let optimised = apply_rules_to_fixed_point(plan, &show_plan_rules());

    print!("{}", format_plan(&optimised));
    Ok(())
}
