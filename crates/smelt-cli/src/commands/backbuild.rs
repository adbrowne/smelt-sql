use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_selector_args},
    backend_factory::CliBackendFactory,
    reporter::CliReporter,
    Config, ModelDiscovery, SourcesConfig,
};
use smelt_core::graph::DependencyGraph;
use smelt_runtime::types::ExecuteRequest;
use smelt_state::generate_run_id;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use tracing::info;

use super::run_setup::*;
use crate::BackbuildArgs;

/// Prefix a plain selector with `+` for upstream-closure unless it already
/// carries an upstream operator. Graph-operator selectors (those already
/// starting with `+`, `tag:`, etc.) pass through unchanged.
///
/// Upstream-closure semantics: `smelt backbuild model_name` rebuilds `model_name`
/// and every upstream model it depends on. This is achieved by rewriting the
/// selector to `+model_name` (the `+prefix` graph operator in the selection
/// DSL means "include all transitive upstreams").
///
/// Selectors already containing `:` (e.g. `tag:foo`) or already prefixed
/// with `+` are passed through unchanged because they either do not name a
/// single model or they already carry operator syntax.
fn to_upstream_closure(selector: &str) -> String {
    if selector.starts_with('+') || selector.contains(':') {
        selector.to_string()
    } else {
        format!("+{}", selector)
    }
}

pub async fn backbuild(args: BackbuildArgs, scope: Option<&str>) -> Result<()> {
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
    let ephemeral_seed_ctes =
        build_ephemeral_seed_ctes(&seeds, resolve_target_backend_type(&config, &args.target)?);

    if models.is_empty() {
        return Err(anyhow::anyhow!(
            "No models found in paths: {}",
            config.paths.join(", ")
        ));
    }
    info!("Found {} models total", models.len());
    check_parse_errors(&models)?;
    validate_materialization_configs(&models, &config)?;

    // 4. Build dependency graph + resolve selector.
    let mut graph = DependencyGraph::build(models.clone(), sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    graph.add_seeds(&seeds);

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

    // Resolve the single selector through scope, then apply upstream-closure
    // rewrite so backbuild always includes all transitive upstreams.
    let resolved_select = resolve_selector_args(
        &gen_salsa_db,
        gen_salsa_ws,
        gen_salsa_project,
        active_scope.as_ref(),
        std::slice::from_ref(&args.selector),
    )
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    let upstream_selectors: Vec<String> = resolved_select
        .iter()
        .map(|s| to_upstream_closure(s))
        .collect();

    // 5. Validate date format.
    chrono::NaiveDate::parse_from_str(&args.start, "%Y-%m-%d").with_context(|| {
        format!(
            "Invalid start date format: {}. Expected YYYY-MM-DD",
            args.start
        )
    })?;
    chrono::NaiveDate::parse_from_str(&args.end, "%Y-%m-%d")
        .with_context(|| format!("Invalid end date format: {}. Expected YYYY-MM-DD", args.end))?;
    info!("Time range: {} to {} (exclusive)", args.start, args.end);

    // 6. Build Salsa DB for execute_project, then run.
    let salsa_db = build_execute_salsa_db(
        &discovery,
        &function_files,
        &models,
        &project_dir,
        &args.target,
    )?;

    // backbuild always passes full_refresh: false — the upstream-closure
    // selector rebuilds upstream table models as full-refreshes (their default)
    // while keyed models receive the per-partition merge loop.
    let request = ExecuteRequest {
        target: args.target.clone(),
        select: upstream_selectors,
        exclude: vec![],
        start: Some(args.start.clone()),
        end: Some(args.end.clone()),
        batch_size_days: args.batch_size,
        per_partition: args.per_partition,
        full_refresh: false,
        dry_run: args.dry_run,
        enforce_safety: !args.allow_downgrade,
        allow_column_removal: false,
        allow_full_refresh: false,
        ephemeral_seed_ctes,
        run_checks: false,
        checks: vec![],
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
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
        info!(
            "[DRY RUN] Compiled {} model(s); no execution performed.",
            planned
        );
        return Ok(());
    }

    info!("{}", "=".repeat(60));
    info!("Backbuild Summary (run: {})", run_id);
    info!("{}", "=".repeat(60));
    info!("Executed {} models successfully", outcome.models.len());
    Ok(())
}
