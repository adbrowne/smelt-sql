use anyhow::{Context, Result};
use smelt_cli::{discover_python_models, find_project_root, init_db, Config, ModelDiscovery};
use smelt_db::TypeChecking;

use crate::TypeArgs;

pub async fn show_type(args: TypeArgs) -> Result<()> {
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

    // 4. Show function types
    if let Some(model_name) = &args.model_name {
        // Show single model
        let model = models
            .iter()
            .find(|m| m.name == *model_name)
            .ok_or_else(|| anyhow::anyhow!("Model '{}' not found", model_name))?;
        let ft = db.model_function_type(model.path.clone());
        println!("{}", ft);
    } else {
        // Show all models sorted by name
        let mut model_list: Vec<_> = models.iter().collect();
        model_list.sort_by(|a, b| a.name.cmp(&b.name));

        for (i, model) in model_list.iter().enumerate() {
            if i > 0 {
                println!();
            }
            let ft = db.model_function_type(model.path.clone());
            println!("{}", ft);
        }
    }

    Ok(())
}
