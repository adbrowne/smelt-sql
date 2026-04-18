use anyhow::{Context, Result};
use smelt_cli::{
    discover_python_models, find_project_root, parse_selector, Config, LogicalGraph,
    ModelDiscovery, SourcesConfig,
};

use crate::DocsGenerateArgs;

pub async fn generate(args: DocsGenerateArgs) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

    // Seeds are valid `smelt.ref()` targets (bug #2 in 20260417 follow-up).
    let seeds = smelt_core::discover_seed_infos(&project_dir, &config.seed_paths);

    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Filter out test models
    models.retain(|m| !m.is_test());

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
        models.extend(python_models);
    }

    let default_target = config
        .targets
        .keys()
        .next()
        .map(|s| s.as_str())
        .unwrap_or("dev");
    let graph = LogicalGraph::build(
        models.clone(),
        sources.as_ref(),
        &seeds,
        &config,
        default_target,
    )
    .with_context(|| "Failed to build logical graph")?;

    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    // Apply --select filters if provided
    let graph = if !args.select.is_empty() {
        let selectors: Vec<_> = args
            .select
            .iter()
            .map(|s| parse_selector(s))
            .collect::<Result<_, _>>()
            .with_context(|| "Failed to parse selector")?;
        let selected = graph.select_models(&selectors)?;
        let filtered_models: Vec<_> = models
            .into_iter()
            .filter(|m| selected.contains(&m.name))
            .collect();
        LogicalGraph::build(
            filtered_models,
            sources.as_ref(),
            &seeds,
            &config,
            default_target,
        )
        .with_context(|| "Failed to build filtered logical graph")?
    } else {
        graph
    };

    // Initialize Salsa DB for type inference
    let db = smelt_cli::init_db(
        &project_dir,
        &graph
            .iter_models()
            .map(|(_, m)| m.clone())
            .collect::<Vec<_>>(),
    );

    let catalog = smelt_cli::docs::build_catalog(&graph, &config, &db)?;

    let output_dir = args
        .output
        .unwrap_or_else(|| project_dir.join("target").join("docs"));

    match args.format.as_str() {
        "json" => {
            smelt_cli::docs_render::render_json(&catalog, &output_dir)?;
            println!("Wrote {}/catalog.json", output_dir.display());
        }
        "markdown" | "md" => {
            smelt_cli::docs_render::render_markdown(&catalog, &output_dir)?;
            println!(
                "Wrote {} model pages to {}/",
                catalog.models.len(),
                output_dir.display()
            );
        }
        other => {
            anyhow::bail!("Unknown format '{}'. Supported: markdown, json", other);
        }
    }

    Ok(())
}
