//! Thin arg-parsing shim for `smelt bakeoff` (`explain.rs` precedent, decision
//! B3): argument resolution + workspace loading only, the measurement engine
//! lives in `smelt_cli::bakeoff::run_bakeoff`.

use std::sync::Arc;

use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_argument},
    bakeoff::{run_bakeoff, BakeoffOptions, CellSelector},
    find_project_root, init_db, Config, ModelDiscovery,
};

use crate::BakeoffArgs;

pub async fn bakeoff(args: BakeoffArgs, scope: Option<&str>) -> Result<()> {
    if args.pin {
        return Err(smelt_cli::bakeoff::pin_not_yet_supported());
    }

    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    let db = init_db(&project_dir, &models);
    let ws = smelt_db::Workspace::try_get(&db)
        .ok_or_else(|| anyhow::anyhow!("workspace not initialized"))?;
    let project = db
        .project_input(&project_dir)
        .ok_or_else(|| anyhow::anyhow!("project not initialized"))?;

    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
    let canonical = resolve_argument(&db, ws, project, active_scope.as_ref(), &args.model_name)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let cells: Vec<CellSelector> = args
        .cells
        .iter()
        .flat_map(|raw| raw.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(CellSelector::parse)
        .collect::<Result<_>>()?;

    let opts = BakeoffOptions {
        cells,
        runs: args.runs,
        target: args.target.clone(),
        keep: args.keep,
    };

    let report = run_bakeoff(&project_dir, Arc::new(config), &canonical, opts).await?;
    println!("{report}");
    Ok(())
}
