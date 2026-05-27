use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_argument},
    discover_python_models, find_project_root, init_db, Config, ModelDiscovery,
};

use crate::TypeArgs;

pub async fn show_type(args: TypeArgs, scope: Option<&str>) -> Result<()> {
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

    // Discover Python models
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

    // 4. Compute active scope
    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);

    // 5. Show function types
    if let Some(model_name) = &args.model_name {
        // Resolve the argument to a canonical path
        let canonical = resolve_argument(&db, ws, project, active_scope.as_ref(), model_name)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        // Look up the model by canonical path
        let model = models
            .iter()
            .find(|m| m.canonical_path() == canonical)
            .ok_or_else(|| anyhow::anyhow!("Model '{}' not found", canonical))?;

        let file = db
            .source_file(&model.path)
            .expect("model file not registered");
        let mut ft = (*smelt_db::model_function_type(&db, ws, file)).clone();
        // Override model_name to canonical path so output prints canonically.
        ft.model_name = canonical;
        println!("{}", ft);
    } else {
        // Show all models sorted by canonical path
        let mut model_list: Vec<_> = models.iter().collect();
        model_list.sort_by_key(|a| a.canonical_path());

        for (i, model) in model_list.iter().enumerate() {
            if i > 0 {
                println!();
            }
            let file = db
                .source_file(&model.path)
                .expect("model file not registered");
            let mut ft = (*smelt_db::model_function_type(&db, ws, file)).clone();
            ft.model_name = model.canonical_path();
            println!("{}", ft);
        }
    }

    Ok(())
}
