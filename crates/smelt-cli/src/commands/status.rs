use anyhow::{Context, Result};
use chrono::Utc;
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_argument},
    find_project_root, init_db, Config, ModelDiscovery,
};
use smelt_state::file_store::FileStore;

use crate::StatusArgs;

pub async fn status(args: StatusArgs, scope: Option<&str>) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let file_store = FileStore::new(&project_dir);
    if !file_store.exists() {
        println!("No state directory found. Run `smelt run` with a time range first.");
        return Ok(());
    }

    let interval_store = file_store
        .load_intervals()
        .with_context(|| "Failed to load interval store")?;

    if interval_store.models.is_empty() {
        println!("No interval data recorded yet.");
        return Ok(());
    }

    let today = Utc::now().format("%Y-%m-%d").to_string();
    let until = args.until.as_deref().unwrap_or(&today);

    // If a model name was given, resolve it via scope.
    let resolved_model_name: Option<String> = if let Some(ref name) = args.model_name {
        // Load config for scope computation.
        let config =
            Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;
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
            Err(_) => Some(name.clone()), // fall back to raw name for interval store lookup
        }
    } else {
        None
    };

    let models_to_show: Vec<(&String, &smelt_state::intervals::ModelIntervals)> =
        if let Some(ref name) = resolved_model_name {
            interval_store
                .get(name)
                .map(|i| vec![(name, i)])
                .unwrap_or_else(|| {
                    tracing::warn!("Model '{}' not found in interval store", name);
                    vec![]
                })
        } else {
            let mut v: Vec<_> = interval_store.models.iter().collect();
            v.sort_by_key(|(k, _)| (*k).clone());
            v
        };

    println!("Interval Coverage Status");
    println!("{}", "=".repeat(60));

    for (model_name, intervals) in &models_to_show {
        println!("\n  {}", model_name);
        println!("  {}", "-".repeat(40));

        if intervals.covered_intervals.is_empty() {
            println!("    No coverage (model hash changed or never run)");
            continue;
        }

        for interval in &intervals.covered_intervals {
            println!("    Covered: {} to {}", interval.start, interval.end);
        }

        if let Some(since) = args.since.as_deref().or(intervals
            .earliest_date()
            .as_ref()
            .map(|_| intervals.covered_intervals[0].start.as_str()))
        {
            let gaps = intervals.find_gaps(since, until);
            if gaps.is_empty() {
                println!("    No gaps in [{}, {})", since, until);
            } else {
                for gap in &gaps {
                    println!("    GAP: {} to {}", gap.start, gap.end);
                }
            }
        }

        println!("    Hash: {}", intervals.model_hash);
    }

    Ok(())
}
