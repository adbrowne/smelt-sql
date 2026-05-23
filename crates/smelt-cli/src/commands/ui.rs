use anyhow::{Context, Result};
use smelt_cli::{find_project_root, init_db, Config, ModelDiscovery, SourcesConfig};

use tracing::info;

use crate::UiArgs;

pub async fn ui(args: UiArgs) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    info!("Found {} models", models.len());

    let function_files = discovery
        .discover_function_files()
        .with_context(|| "Failed to discover function files")?;

    // Initialize smelt-db for schema queries; include function files so
    // smelt.functions.* calls resolve without "Unknown function" diagnostics.
    let mut db_files: Vec<smelt_core::ModelFile> = models.to_vec();
    db_files.extend(function_files);
    let db = init_db(&project_dir, &db_files);

    // Build dependency graph. `models` is `Vec<smelt_core::ModelFile>` so
    // the previous field-by-field rebuild is redundant since the type was
    // unified across CLI and core; just clone.
    let core_models: Vec<smelt_core::ModelFile> = models.to_vec();
    let graph = smelt_core::graph::DependencyGraph::build(core_models, sources.as_ref())
        .with_context(|| "Failed to build UI dependency graph")?;

    smelt_ui::start_server(
        db,
        config,
        sources,
        graph,
        project_dir,
        args.port,
        &args.host,
    )
    .await
}
