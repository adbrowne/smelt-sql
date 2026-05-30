use crate::helpers::{print_json, print_table};
use crate::TableArgs;
use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_argument},
    discover_python_models, find_project_root, init_db, Config, ModelDiscovery,
};

pub async fn table(args: TableArgs, scope: Option<&str>) -> Result<()> {
    // 1. Find project root
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    // 2. Load configuration and discover models
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
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
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let project = db
        .project_input(&project_dir)
        .expect("project not initialized");

    // 4. Compute active scope and resolve the model argument
    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
    let canonical = resolve_argument(&db, ws, project, active_scope.as_ref(), &args.model_name)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // 5. Find the model by canonical path
    let model = models
        .iter()
        .find(|m| m.canonical_path() == canonical)
        .ok_or_else(|| anyhow::anyhow!("Model '{}' not found", canonical))?;

    // 6. Get typed schema
    let file = db
        .source_file(&model.path)
        .expect("model file not registered");
    let schema = smelt_db::typed_model_schema(&db, ws, file);

    // 7. Output using canonical path as the model name
    match args.format.as_str() {
        "json" => print_json(&schema, &canonical),
        _ => print_table(&schema, &canonical),
    }

    Ok(())
}
