use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_selector_args},
    find_project_root, seed, Config,
};

use tracing::info;

use crate::helpers::create_backend;
use crate::SeedArgs;

pub async fn run_seed(args: SeedArgs, scope: Option<&str>) -> Result<()> {
    // 1. Find project root and load config
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    info!("Project directory: {}", project_dir.display());

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    info!("Project: {} (version {})", config.name, config.version);

    // 2. Get target config
    let target_config = config.targets.get(&args.target).ok_or_else(|| {
        anyhow::anyhow!(
            "Target '{}' not found in smelt.yml. Available targets: {}",
            args.target,
            config
                .targets
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    // 3. Discover seeds
    let mut seeds = seed::discover_seeds(&project_dir, &config.paths, &target_config.schema)
        .with_context(|| "Failed to discover seeds")?;

    if seeds.is_empty() {
        info!("No seed files found in: {}", config.paths.join(", "));
        return Ok(());
    }

    // 4. Filter by --select if provided.
    // Route through resolve_selector_args so bare-leaf with no scope errors per
    // spec (instead of silently passing the raw string to a substring filter).
    if !args.select.is_empty() {
        // Build a lightweight Salsa DB (no SQL model files) so resolve_argument
        // can call smelt_db::resolve_ref_path, which finds seeds via project_seeds.
        let mut db = smelt_db::Database::default();
        let project = db.set_project_input(project_dir.clone(), String::new());
        db.set_workspace(vec![], vec![project]);
        let ws = db.workspace();

        // Compute the active scope.
        let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
        let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);

        // Resolve selectors → canonical seed paths (address_segments joined by ".").
        let resolved_select =
            resolve_selector_args(&db, ws, project, active_scope.as_ref(), &args.select)
                .map_err(|e| anyhow::anyhow!("{}", e))?;

        // Keep only seeds whose canonical path matches a resolved selector.
        seeds.retain(|s| {
            let canonical = s.address_segments.join(".");
            resolved_select.contains(&canonical)
        });

        if seeds.is_empty() {
            info!("No seeds matched selectors: {}", args.select.join(", "));
            return Ok(());
        }
    }

    info!("Found {} seed(s)", seeds.len());

    // 5. Create backend
    let backend = create_backend(target_config, &project_dir, args.database).await?;

    // 6. Execute seeds
    info!("{}", "=".repeat(60));
    info!("Seeding...");
    info!("{}", "=".repeat(60));

    let mut results = Vec::new();

    for s in &seeds {
        info!("Seeding: {}", s.qualified_name());

        match seed::execute_seed(backend.as_ref(), s, args.show_results)
            .await
            .with_context(|| format!("Failed to seed '{}'", s.qualified_name()))?
        {
            None => {
                // Ephemeral seed — nothing to load.
                info!("{} skipped (ephemeral)", s.qualified_name());
            }
            Some(result) => {
                info!(
                    "{} done ({} rows, {:?})",
                    result.qualified_name, result.row_count, result.duration
                );
                results.push(result);
            }
        }
    }

    // 7. Summary
    info!("{}", "=".repeat(60));
    info!("Summary");
    info!("{}", "=".repeat(60));
    info!("Loaded {} seed(s) successfully", results.len());

    let total_rows: usize = results.iter().map(|r| r.row_count).sum();
    let total_duration: std::time::Duration = results.iter().map(|r| r.duration).sum();
    info!("Total rows: {}", total_rows);
    info!("Total time: {:?}", total_duration);

    eprintln!(
        "smelt: loaded {} seed(s) ({} rows) in {:.2}s",
        results.len(),
        total_rows,
        total_duration.as_secs_f64(),
    );

    Ok(())
}
