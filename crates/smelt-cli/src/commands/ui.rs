use anyhow::{Context, Result};
use smelt_cli::{find_project_root, init_db, Config, ModelDiscovery, SourcesConfig};

use crate::UiArgs;

pub async fn ui(args: UiArgs) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    println!("Found {} models", models.len());

    // Initialize smelt-db for schema queries
    let db = init_db(&project_dir, &models);

    // Build dependency graph
    let core_models: Vec<smelt_core::ModelFile> = models
        .iter()
        .map(|m| smelt_core::ModelFile {
            name: m.name.clone(),
            path: m.path.clone(),
            content: m.content.clone(),
            refs: m.refs.clone(),
            parse_errors: m.parse_errors.clone(),
            metadata: m.metadata.clone(),
            model_id: m.model_id.clone(),
        })
        .collect();
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
