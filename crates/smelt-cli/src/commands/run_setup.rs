//! Setup helpers for `commands/run.rs`.
//!
//! Extracted so `run.rs` stays < 250 lines and the per-step logic is readable
//! in isolation. All functions here are private to the `commands` module.

use anyhow::{Context, Result};
use chrono::Utc;
use smelt_cli::{
    discover_emitted_model_files, discover_python_models, executor, init_db, Config, ModelDiscovery,
    ModelFile, SourcesConfig,
};
use smelt_core::graph::DependencyGraph;
use smelt_state::file_store::FileStore;
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

/// Discover SQL + emitted + Python models, filtered to the working set
/// (no `.gen.` files, no test models).  Returns `(models, function_files)`.
pub(super) fn discover_models_for_run(
    discovery: &ModelDiscovery,
    target: &str,
    project_dir: &Path,
    config: &Config,
) -> Result<(Vec<ModelFile>, Vec<smelt_cli::ModelFile>)> {
    let raw_sql_models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    let mut gen_salsa_db = init_db(project_dir, &raw_sql_models);
    gen_salsa_db.set_active_target(Some(std::sync::Arc::from(target)));

    let (emitted_model_files, _origins) =
        discover_emitted_model_files(&gen_salsa_db, project_dir, &config.paths);
    if !emitted_model_files.is_empty() {
        info!(
            "Generator pipeline produced {} emitted model(s)",
            emitted_model_files.len()
        );
    }

    let mut models: Vec<ModelFile> = raw_sql_models
        .into_iter()
        .filter(|m| !m.name.ends_with(".gen") && !m.path.to_string_lossy().contains(".gen."))
        .filter(|m| !m.is_test())
        .collect();
    models.extend(emitted_model_files);

    let python_files = discovery
        .discover_python_files()
        .with_context(|| "Failed to scan for Python models")?;

    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            config,
            project_dir,
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

    let function_files = discovery
        .discover_function_files()
        .with_context(|| "Failed to discover function files")?;

    Ok((models, function_files))
}

/// Build ephemeral-seed CTE triples from discovered seeds.
pub(super) fn build_ephemeral_seed_ctes(
    seeds: &[smelt_core::SeedInfo],
) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for seed in seeds.iter().filter(|s| s.is_ephemeral()) {
        if let Ok((_headers, rows_iter)) = smelt_core::seeds::csv::read_csv(&seed.path) {
            let rows: Vec<_> = rows_iter.filter_map(|r| r.ok()).collect();
            let canonical_name = seed.address_segments.join("_");
            let col_names: Vec<&str> = seed.columns.iter().map(|(n, _)| n.as_str()).collect();
            let alias_with_cols = format!("__smelt_{}({})", canonical_name, col_names.join(", "));
            let cte_body = smelt_core::seeds::ephemeral::build_values_cte(&seed.columns, &rows);
            out.push((canonical_name, alias_with_cols, cte_body));
        }
    }
    out
}

/// Surface parse errors to stderr and return `Err` if any were found.
pub(super) fn check_parse_errors(models: &[ModelFile]) -> Result<()> {
    let mut error_names: Vec<String> = Vec::new();
    for model in models {
        if !model.parse_errors.is_empty() {
            error_names.push(model.name.clone());
            tracing::error!("Parse errors in {}:", model.name);
            for err in &model.parse_errors {
                tracing::error!("  - {} at {:?}", err.message, err.range);
            }
            eprintln!("error: parse errors in model '{}':", model.name);
            for err in &model.parse_errors {
                eprintln!("  - {} at {:?}", err.message, err.range);
            }
        }
    }
    if !error_names.is_empty() {
        return Err(anyhow::anyhow!(
            "Parse errors in {} model(s): {}",
            error_names.len(),
            error_names.join(", "),
        ));
    }
    Ok(())
}

/// Validate materialization configs for the discovered model set.
pub(super) fn validate_materialization_configs(
    models: &[ModelFile],
    config: &Config,
) -> Result<()> {
    let metadata_map: HashMap<String, smelt_cli::ModelMetadata> = models
        .iter()
        .filter_map(|m| {
            m.metadata
                .as_ref()
                .map(|md| (m.name.clone(), md.as_ref().clone()))
        })
        .collect();
    let errors = config.validate_model_configs(&metadata_map);
    if !errors.is_empty() {
        for (model, msg) in &errors {
            tracing::error!("model '{}': {}", model, msg);
        }
        return Err(anyhow::anyhow!(
            "Model configuration validation failed ({} error(s))",
            errors.len()
        ));
    }
    Ok(())
}

/// Compute the `(start, end)` date strings for `--auto` mode.
/// Returns `None` if no interval history is found.
pub(super) fn compute_auto_time_range(
    project_dir: &Path,
    graph: &DependencyGraph,
) -> Option<(String, String)> {
    let file_store = FileStore::new(project_dir);
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
        info!("[AUTO] Time range: {} to {} (from interval store)", start, today);
        Some((start, today))
    } else {
        info!("[AUTO] No interval history found. Use --start/--end for initial run.");
        None
    }
}

/// Run source validation against the active backend if sources are present
/// and no selector is active. Logs warnings on failure; never fails the run.
pub(super) async fn validate_sources_optional(
    sources: Option<&SourcesConfig>,
    config: &Config,
    target: &str,
    database_override: Option<std::path::PathBuf>,
    resolved_select: &[String],
    resolved_exclude: &[String],
    project_dir: &Path,
) {
    let Some(source_config) = sources else { return };
    if !resolved_select.is_empty() || !resolved_exclude.is_empty() {
        return;
    }
    let needed_target: std::collections::HashSet<String> =
        std::iter::once(target.to_string()).collect();
    match smelt_cli::BackendRegistry::new(
        &config.targets,
        &needed_target,
        project_dir,
        database_override,
    )
    .await
    {
        Ok(registry) => {
            if let Err(e) = executor::validate_sources(registry.get(target), source_config).await {
                warn!("Source validation failed: {}", e);
            }
        }
        Err(e) => {
            tracing::debug!("Could not validate sources (backend not ready): {}", e);
        }
    }
}

/// Build the Salsa DB used by `execute_project`. Includes raw SQL models,
/// function files, and any Python or other non-generator models.
pub(super) fn build_execute_salsa_db(
    discovery: &ModelDiscovery,
    function_files: &[ModelFile],
    models: &[ModelFile],
    project_dir: &Path,
    target: &str,
) -> Result<smelt_db::Database> {
    let raw_sql_models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models for salsa DB")?;
    let mut all_files = raw_sql_models;
    all_files.extend(function_files.iter().cloned());
    for m in models {
        let is_raw = all_files.iter().any(|f| f.path == m.path);
        if !is_raw && !m.path.to_string_lossy().ends_with(".gen.sql") {
            all_files.push(m.clone());
        }
    }
    let mut salsa_db = init_db(project_dir, &all_files);
    salsa_db.set_active_target(Some(std::sync::Arc::from(target)));
    Ok(salsa_db)
}
