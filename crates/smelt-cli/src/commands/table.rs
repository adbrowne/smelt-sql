use crate::helpers::{print_json, print_table};
use crate::TableArgs;
use anyhow::{Context, Result};
use smelt_cli::{discover_python_models, find_project_root, init_db, Config, ModelDiscovery};

pub async fn table(args: TableArgs) -> Result<()> {
    // 1. Find project root
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    // 2. Load configuration and discover models
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Discover Python models for table command too
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

    // 3. Initialize Salsa database
    let db = init_db(&project_dir, &models);

    // 4. Find the model path
    let model = models
        .iter()
        .find(|m| m.name == args.model_name)
        .ok_or_else(|| anyhow::anyhow!("Model '{}' not found", args.model_name))?;

    // 5. Get typed schema
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let file = db
        .source_file(&model.path)
        .expect("model file not registered");
    let schema = smelt_db::typed_model_schema(&db, ws, file);

    // 6. Output
    match args.format.as_str() {
        "json" => print_json(&schema, &args.model_name),
        _ => print_table(&schema, &args.model_name),
    }

    Ok(())
}
