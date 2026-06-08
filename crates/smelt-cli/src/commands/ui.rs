use anyhow::{Context, Result};
use smelt_cli::{
    discover_emitted_model_files, find_project_root, init_db, Config, ModelDiscovery, SourcesConfig,
};

use tracing::info;

use crate::UiArgs;

pub async fn ui(args: UiArgs) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let raw_models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    info!("Found {} raw models", raw_models.len());

    let function_files = discovery
        .discover_function_files()
        .with_context(|| "Failed to discover function files")?;

    // Initialize smelt-db with raw SQL + function files so the meta-language
    // evaluator can expand generator bodies and smelt.functions.* calls resolve.
    let mut db_files: Vec<smelt_core::ModelFile> = raw_models.to_vec();
    db_files.extend(function_files.iter().cloned());
    let db = init_db(&project_dir, &db_files);

    // Expand generator files: discover the models they emit and add them to the
    // graph alongside the hand-authored models (generator files themselves are
    // filtered out — their bodies are meta-language, not executable SQL).
    // This mirrors what `discover_models_for_run` does for CLI runs; without it
    // generator-emitted models would be silently absent from every UI run.
    let (emitted, _origins) = discover_emitted_model_files(&db, &project_dir, &config.paths);
    if !emitted.is_empty() {
        info!(
            "Generator pipeline produced {} emitted model(s)",
            emitted.len()
        );
    }

    let mut core_models: Vec<smelt_core::ModelFile> = raw_models
        .into_iter()
        .filter(|m| m.metadata.as_ref().is_none_or(|md| md.generates.is_none()))
        .collect();
    core_models.extend(emitted);

    info!(
        "Building dependency graph with {} model(s)",
        core_models.len()
    );
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
