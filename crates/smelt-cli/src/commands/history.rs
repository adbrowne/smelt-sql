use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_argument},
    find_project_root, init_db, Config, ModelDiscovery,
};
use smelt_state::file_store::FileStore;
use smelt_state::history::HistoryQuery;

use crate::HistoryArgs;

pub async fn history(args: HistoryArgs, scope: Option<&str>) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;
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

    let file_store = FileStore::new(&project_dir, &args.target);
    let manifests = file_store
        .load_runs(Some(args.limit))
        .with_context(|| "Failed to load run history")?;

    if manifests.is_empty() {
        if config.state.mode == smelt_core::config::StateMode::Stateless {
            // stdout: explains to the human why `smelt history` has nothing to show
            println!(
                "No run history: target '{}' is running with state.mode: stateless, which \
                 writes no run history. Set state.mode to intervals or environments to enable \
                 history tracking.",
                args.target
            );
        } else {
            println!("No run history found.");
        }
        return Ok(());
    }

    let query = HistoryQuery::new(&manifests);

    // If a model name was given, resolve it via scope.
    let resolved_model_name: Option<String> = if let Some(ref name) = args.model_name {
        let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
        let models = discovery
            .discover_models()
            .with_context(|| "Failed to discover models")?;
        let db = init_db(&project_dir, &models);
        let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
        let project = db
            .project_input(&project_dir)
            .expect("project not initialized");
        let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
        let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
        match resolve_argument(&db, ws, project, active_scope.as_ref(), name) {
            Ok(canonical) => Some(canonical),
            Err(_) => Some(name.clone()), // fall back to raw name for history lookup
        }
    } else {
        None
    };

    if let Some(ref model_name) = resolved_model_name {
        let summaries = query.for_model(model_name);
        if summaries.is_empty() {
            println!("No runs found for model '{}'", model_name);
            return Ok(());
        }

        println!("Run History for '{}'", model_name);
        println!("{}", "=".repeat(60));

        for (i, summary) in summaries.iter().take(args.limit).enumerate() {
            println!(
                "\n  {}. Run: {} ({})",
                i + 1,
                summary.run_id,
                summary.started_at
            );
            println!("     Strategy: {}", summary.strategy);
            if let Some((start, end)) = &summary.time_range {
                println!("     Range: {} to {}", start, end);
            }
            println!(
                "     {} rows in {}ms",
                summary.row_count, summary.duration_ms
            );
        }
    } else {
        let runs = query.last_n(args.limit);

        println!("Run History");
        println!("{}", "=".repeat(60));

        for (i, manifest) in runs.iter().enumerate() {
            let model_count = manifest.models.len();
            let total_rows: usize = manifest.models.values().map(|m| m.row_count).sum();
            let total_ms: u64 = manifest.models.values().map(|m| m.duration_ms).sum();
            let completed = manifest
                .completed_at
                .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "in progress".to_string());

            println!(
                "\n  {}. {} (completed: {})",
                i + 1,
                manifest.run_id,
                completed
            );
            println!(
                "     {} model(s), {} total rows, {}ms",
                model_count, total_rows, total_ms
            );

            for (name, record) in &manifest.models {
                let range_str = record
                    .time_range
                    .as_ref()
                    .map(|tr| format!(" [{}, {})", tr.start, tr.end))
                    .unwrap_or_default();
                println!(
                    "       {} ({}){} → {} rows",
                    name, record.strategy, range_str, record.row_count
                );
            }
        }
    }

    println!();
    Ok(())
}
