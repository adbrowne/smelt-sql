use anyhow::{Context, Result};
use chrono::Utc;
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_selector_args},
    discover_emitted_model_files, discover_python_models, executor, find_project_root, init_db,
    parse_selector, Config, ModelDiscovery, SourcesConfig,
};
use smelt_core::graph::DependencyGraph;
use smelt_runtime::reporter::RunReporter;
use smelt_runtime::types::ExecuteRequest;
use smelt_state::file_store::FileStore;
use smelt_state::generate_run_id;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use tracing::{info, warn};

use crate::RunArgs;
use smelt_cli::backend_factory::CliBackendFactory;

/// Minimal CLI reporter that forwards runtime events to `tracing` / stderr.
///
/// When `dry_run` is true, `model_compiled` prints "Would run: {model}" followed
/// by the compiled SQL (the runtime compiles models before returning from
/// `execute_project` when `dry_run = true`, so the inline compile block in
/// run.rs is not needed — parity rule enforced).
/// When `verbose` is true (non-dry-run), `model_compiled` prints the compiled
/// SQL with a comment header.
struct CliReporter {
    verbose: bool,
    dry_run: bool,
    #[allow(dead_code)] // Phase 4 will add --show-results support via this field
    show_results: bool,
    model_count: std::sync::atomic::AtomicUsize,
}

impl CliReporter {
    fn new(verbose: bool, dry_run: bool, show_results: bool) -> Self {
        Self {
            verbose,
            dry_run,
            show_results,
            model_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl RunReporter for CliReporter {
    fn run_started(&self, _run_id: &str, models: &[String], _total_batches: usize) {
        self.model_count
            .store(models.len(), std::sync::atomic::Ordering::Relaxed);
        info!("Running {} model(s)…", models.len());
    }

    fn model_started(&self, _run_id: &str, model: &str, idx: usize, total: usize) {
        info!("Running model: {} ({}/{})", model, idx + 1, total);
    }

    fn batch_completed(
        &self,
        _run_id: &str,
        _model: &str,
        batch_idx: usize,
        batches_total: usize,
        row_count: usize,
        duration: Duration,
    ) {
        if batches_total > 1 {
            info!(
                "  batch {}/{}: {} rows ({:?})",
                batch_idx + 1,
                batches_total,
                row_count,
                duration
            );
        }
    }

    fn model_compiled(&self, _run_id: &str, model: &str, sql: &str) {
        if self.dry_run {
            // Dry-run: print "Would run: <model>" prefix + compiled SQL.
            // SQL arrives here because execute_project compiles models and calls
            // this callback before returning — no second compile pass needed.
            println!("-- Would run: {} (materialization shown below)", model);
            if !sql.is_empty() {
                println!("{}", sql.trim_end());
            }
            println!();
        } else if self.verbose {
            // Use println! (not tracing!) so the SQL surfaces without requiring
            // RUST_LOG=debug. See `docs/specs/cli.md` §"`--verbose`".
            println!("-- {}", model);
            println!("{}", sql);
            println!();
        }
    }

    fn model_completed(&self, _run_id: &str, model: &str, row_count: usize, duration: Duration) {
        info!("{} done ({} rows, {:?})", model, row_count, duration);
    }

    fn run_completed(&self, _run_id: &str, _total_rows: usize, duration: Duration) {
        let count = self.model_count.load(std::sync::atomic::Ordering::Relaxed);
        // Mirror a one-line summary to stderr unconditionally so users without
        // `RUST_LOG` configured see something on success. Matches the format
        // the old execute loop emitted ("smelt: built N model(s)").
        eprintln!(
            "smelt: built {} model(s) in {:.2}s",
            count,
            duration.as_secs_f64(),
        );
    }

    fn run_failed(&self, _run_id: &str, model: Option<&str>, error: &str) {
        if let Some(m) = model {
            eprintln!("smelt: run failed at model '{}': {}", m, error);
        } else {
            eprintln!("smelt: run failed: {}", error);
        }
    }

    fn run_cancelled(&self, _run_id: &str) {
        eprintln!("smelt: run cancelled");
    }
}

pub async fn run(args: RunArgs, scope: Option<&str>) -> Result<()> {
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

    // Discover seeds (valid ref targets)
    let seeds = smelt_core::discover_seed_infos_with_sidecars(&project_dir, &config.paths);
    if !seeds.is_empty() {
        info!("Discovered {} seed(s) as ref targets", seeds.len());
    }

    // 3. Discover models (SQL + Python)
    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let raw_sql_models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Initialise a Salsa DB from ALL discovered files (including generator files).
    let mut gen_salsa_db = init_db(&project_dir, &raw_sql_models);
    gen_salsa_db.set_active_target(Some(std::sync::Arc::from(args.target.as_str())));

    // Expand generator files into virtual ModelFile entries.
    let (emitted_model_files, _origins) =
        discover_emitted_model_files(&gen_salsa_db, &project_dir, &config.paths);
    if !emitted_model_files.is_empty() {
        info!(
            "Generator pipeline produced {} emitted model(s)",
            emitted_model_files.len()
        );
    }

    // Build the working model list (no generator files, no test models, add emitted).
    let mut models: Vec<smelt_cli::ModelFile> = raw_sql_models
        .into_iter()
        .filter(|m| !m.name.ends_with(".gen") && !m.path.to_string_lossy().contains(".gen."))
        .filter(|m| !m.is_test())
        .collect();
    models.extend(emitted_model_files);

    // Ephemeral seeds: CSV files that have `materialization: ephemeral` in their sidecar YAML.
    // Build pre-formatted CTE triples for `ExecuteRequest::ephemeral_seed_ctes`.
    let mut ephemeral_seed_ctes: Vec<(String, String, String)> = Vec::new();
    for seed in seeds.iter().filter(|s| s.is_ephemeral()) {
        if let Ok((_headers, rows_iter)) = smelt_core::seeds::csv::read_csv(&seed.path) {
            let rows: Vec<_> = rows_iter.filter_map(|r| r.ok()).collect();
            let canonical_name = seed.address_segments.join("_");
            let col_names: Vec<&str> = seed.columns.iter().map(|(n, _)| n.as_str()).collect();
            let alias_with_cols = format!("__smelt_{}({})", canonical_name, col_names.join(", "));
            let cte_body = smelt_core::seeds::ephemeral::build_values_cte(&seed.columns, &rows);
            ephemeral_seed_ctes.push((canonical_name, alias_with_cols, cte_body));
        }
    }

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
            "No models found in paths: {}",
            config.paths.join(", ")
        ));
    }

    info!("Found {} models total", models.len());

    // Surface parse errors as a hard failure.
    let mut parse_error_models: Vec<String> = Vec::new();
    for model in &models {
        if !model.parse_errors.is_empty() {
            parse_error_models.push(model.name.clone());
            tracing::error!("Parse errors in {}:", model.name);
            for error in &model.parse_errors {
                tracing::error!("  - {} at {:?}", error.message, error.range);
            }
            eprintln!("error: parse errors in model '{}':", model.name);
            for error in &model.parse_errors {
                eprintln!("  - {} at {:?}", error.message, error.range);
            }
        }
    }
    if !parse_error_models.is_empty() {
        return Err(anyhow::anyhow!(
            "Parse errors in {} model(s): {}",
            parse_error_models.len(),
            parse_error_models.join(", "),
        ));
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

    // Get workspace and project from the Salsa DB for scope resolution.
    let gen_salsa_ws =
        smelt_db::Workspace::try_get(&gen_salsa_db).expect("workspace not initialized");
    let gen_salsa_project = gen_salsa_db
        .project_input(&project_dir)
        .expect("project not initialized");

    // Build dependency graph.
    let mut graph = DependencyGraph::build(models.clone(), sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    // Seeds are valid `smelt.ref()` targets; add them so validate() doesn't
    // flag them as undefined.
    graph.add_seeds(&seeds);

    graph.warn_unused_ephemerals(&config);

    // Determine --select / --exclude (resolved through scope)
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

    // Check that directly-selected ephemeral models are rejected (not runnable directly).
    if !resolved_select.is_empty() {
        let selectors: Vec<_> = resolved_select
            .iter()
            .map(|s| parse_selector(s).with_context(|| format!("Invalid selector '{}'", s)))
            .collect::<Result<_, _>>()?;
        for selector in &selectors {
            if let smelt_cli::SelectionMethod::ModelName(name) = &selector.method {
                if !selector.include_upstream && !selector.include_downstream {
                    if let Ok(model) = graph.get_model(name) {
                        let metadata = model.metadata.as_deref();
                        let mat = config.get_materialization_with_metadata(name, metadata);
                        if mat == smelt_core::config::Materialization::Ephemeral {
                            return Err(anyhow::anyhow!(
                                "Cannot run ephemeral model '{}' directly — ephemeral models are inlined as CTEs into downstream models.",
                                name
                            ));
                        }
                    }
                }
            }
        }
    }

    // Build ExecuteRequest from CLI args.
    let effective_start = args.start.as_ref().or(args.event_time_start.as_ref());
    let effective_end = args.end.as_ref().or(args.event_time_end.as_ref());

    // Validate date format if provided.
    if let (Some(start), Some(end)) = (effective_start, effective_end) {
        chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d").with_context(|| {
            format!("Invalid start date format: {}. Expected YYYY-MM-DD", start)
        })?;
        chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d")
            .with_context(|| format!("Invalid end date format: {}. Expected YYYY-MM-DD", end))?;
        info!("Time range: {} to {} (exclusive)", start, end);
    } else if effective_start.is_some() != effective_end.is_some() {
        return Err(anyhow::anyhow!(
            "Both --start and --end (or --event-time-start and --event-time-end) must be provided together"
        ));
    }

    // Handle --auto mode: compute time range from interval store.
    let (auto_start, auto_end) =
        if args.auto && effective_start.is_none() && effective_end.is_none() {
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

            // Use execution_order to enumerate incremental models.
            let exec_order = graph.execution_order().unwrap_or_default();
            let mut latest: Option<chrono::NaiveDate> = None;
            for model_name in &exec_order {
                if let Some(intervals) = interval_store.get(model_name) {
                    if let Some(date) = intervals.latest_date() {
                        latest = Some(match latest {
                            Some(prev) => prev.min(date),
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
                (Some(start), Some(today))
            } else {
                info!("[AUTO] No interval history found. Use --start/--end for initial run.");
                (None, None)
            }
        } else {
            (None, None)
        };

    let start_val = effective_start.cloned().or(auto_start);
    let end_val = effective_end.cloned().or(auto_end);

    // Validate sources if present and no selector active.
    if let Some(ref source_config) = sources {
        if resolved_select.is_empty() && resolved_exclude.is_empty() {
            // Source validation requires a live backend. We do a quick check here
            // using BackendRegistry. This stays in place until Phase 4's
            // CliBackendFactory makes this available through execute_project.
            let needed_target: std::collections::HashSet<String> =
                std::iter::once(args.target.clone()).collect();
            match smelt_cli::BackendRegistry::new(
                &config.targets,
                &needed_target,
                &project_dir,
                args.database.clone(),
            )
            .await
            {
                Ok(registry) => {
                    if let Err(e) =
                        executor::validate_sources(registry.get(&args.target), source_config).await
                    {
                        warn!("Source validation failed: {}", e);
                    }
                }
                Err(e) => {
                    tracing::debug!("Could not validate sources (backend not ready): {}", e);
                }
            }
        }
    }

    // Build the Salsa DB for execute_project.
    // We use raw_sql_models (includes generator .gen.sql files) as the base,
    // then register function files. This ensures generator-emitted refs
    // (e.g. smelt.cohorts.us_west) resolve through the emitted_models Salsa
    // query inside execute_project's diagnostic gate.
    // The emitted model files are NOT directly registered (they are virtual);
    // instead they are produced by the emitted_models Salsa query from the .gen.sql.
    let raw_sql_models_for_db = discovery
        .discover_models()
        .with_context(|| "Failed to discover models for salsa DB")?;
    let mut all_files_for_db = raw_sql_models_for_db;
    all_files_for_db.extend(function_files.iter().cloned());
    // Also add non-generator, non-test regular models (Python models etc.)
    // that were discovered but aren't in raw_sql_models.
    for m in &models {
        let is_raw = all_files_for_db.iter().any(|f| f.path == m.path);
        if !is_raw && !m.path.to_string_lossy().ends_with(".gen.sql") {
            all_files_for_db.push(m.clone());
        }
    }
    let mut salsa_db = init_db(&project_dir, &all_files_for_db);
    salsa_db.set_active_target(Some(std::sync::Arc::from(args.target.as_str())));

    // Note: The diagnostic-parity gate is now handled inside execute_project
    // (the gate runs over the selected models + function/generator files using
    // the salsa_db passed in). No pre-gate is needed here.

    // Build the ExecuteRequest.
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
    let cancel = CancellationToken::new();
    let backend_factory = CliBackendFactory {
        database_override: args.database,
    };
    let reporter = CliReporter::new(args.verbose, args.dry_run, args.show_results);

    let outcome = smelt_runtime::execute_project(
        run_id.clone(),
        request,
        config_arc,
        graph_arc,
        db_arc,
        &project_dir,
        &backend_factory,
        &reporter,
        cancel,
    )
    .await?;

    // Dry-run: the compiled SQL was already emitted via reporter.model_compiled
    // (execute_project compiles each model and calls the callback before returning
    // when dry_run=true). No second compile pass needed — parity rule enforced.
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

    // --show-plan: print the resolved execution plan summary.
    if args.show_plan {
        if let Some(ref plan) = outcome.plan_summary {
            println!("Execution plan:");
            for record in &plan.models {
                let strategy_label = format_strategy(&record.strategy);
                println!("  {} [{}]", record.name, strategy_label);
            }
        }
    }

    // Print summary
    info!("{}", "=".repeat(60));
    info!("Summary (run: {})", run_id);
    info!("{}", "=".repeat(60));
    info!("Executed {} models successfully", outcome.models.len());

    let _ = (run_start, std::time::Instant::now().elapsed()); // suppress unused warning

    Ok(())
}

/// Format a `ModelStrategy` as a short human-readable label for `--show-plan` output.
fn format_strategy(strategy: &smelt_runtime::types::ModelStrategy) -> String {
    use smelt_runtime::types::ModelStrategy;
    match strategy {
        ModelStrategy::FullRefresh => "full-refresh".to_string(),
        ModelStrategy::Incremental {
            partition_column,
            granularity,
        } => format!("incremental (by {}, {})", partition_column, granularity),
        ModelStrategy::Cumulative => "cumulative".to_string(),
        ModelStrategy::Ephemeral => "ephemeral".to_string(),
        ModelStrategy::Skipped { .. } => "skipped".to_string(),
    }
}
