//! `smelt state seed-interval` — bootstrap tool that writes one model's
//! interval history directly, offline, with no backend connection.
//!
//! Exists for a target with no local run history to inherit (a
//! Volume-resident scheduled-job store), so `--auto`'s frontier detection
//! (`run_setup::compute_auto_time_range`) has a starting point derived from
//! data already known to be ingested rather than refusing on an empty
//! `intervals.json`
//! (`docs/outcomes/20260912-databricks-dogfood-spine/phases/11j-plan.md`).
//! Seeding is a one-time bootstrap, not an ongoing reconciliation path: once
//! seeded, the target's real run pipeline becomes the sole writer.

use anyhow::{Context, Result};
use smelt_cli::{
    build_fn_body_map_from_model_files, find_project_root, init_db, CompilerRegistry, Config,
    Materialization, ModelDiscovery,
};
use smelt_core::graph::DependencyGraph;
use smelt_state::intervals::{compute_model_hash, seed_interval};
use std::collections::HashMap;

use crate::SeedIntervalArgs;

pub async fn seed_interval_cmd(args: SeedIntervalArgs) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;
    let Some(target_config) = config.targets.get(&args.target) else {
        return Err(anyhow::anyhow!(
            "Target '{}' not found in smelt.yml. Available targets: {}",
            args.target,
            config
                .targets
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    let schema = target_config.schema.clone();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;
    models.retain(|m| !m.is_assertion());

    let _db = init_db(&project_dir, &models);

    let graph = DependencyGraph::build(models.clone(), None)
        .with_context(|| "Failed to build dependency graph")?;
    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    let model = graph.get_model(&args.model).map_err(|_| {
        anyhow::anyhow!(
            "Model '{}' not found in project. Available models: {}",
            args.model,
            graph
                .iter_models()
                .map(|(name, _)| name.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    // Ephemeral models this model may `smelt.ref()` — mirrors the
    // ephemeral-collection logic `execute_project` and `smelt check` use
    // (iterate topological order, collect models whose materialization is
    // Ephemeral).
    let exec_order = graph.execution_order()?;
    let mut ephemeral_models: Vec<(String, String)> = Vec::new();
    for name in &exec_order {
        if let Ok(m) = graph.get_model(name) {
            let mat = config.get_materialization_with_metadata(name, m.metadata.as_deref());
            if mat == Materialization::Ephemeral {
                ephemeral_models.push((m.db_name_owned(), m.content.clone()));
            }
        }
    }

    // Sanctioned run-pipeline-parity compile path (`docs/specs/architecture.md`
    // §"Run pipeline parity rule (CLI ↔ UI)"): never connects to a backend,
    // only reads `smelt.yml` target metadata.
    let mut targets_map: HashMap<String, smelt_core::config::Target> = HashMap::new();
    targets_map.insert(args.target.clone(), target_config.clone());
    let mut compilers = CompilerRegistry::new(&config, &targets_map)?;

    let fn_files = discovery.discover_function_files().unwrap_or_default();
    let fn_body_map = build_fn_body_map_from_model_files(&fn_files);
    if !fn_body_map.is_empty() {
        compilers.set_function_bodies_all(fn_body_map);
    }

    let compiler = compilers.get(&args.target);
    let resolver = compiler.build_ephemeral_resolver(&ephemeral_models, &schema)?;
    let compiled =
        compiler.compile_with_sql_and_ephemerals(model, &schema, &model.content, &resolver)?;
    let model_hash = compute_model_hash(&compiled.sql);

    seed_interval(
        &project_dir,
        &args.target,
        &model.canonical_path(),
        &model_hash,
        &args.start,
        &args.end,
    )?;

    let intervals_path = project_dir
        .join(".smelt")
        .join("targets")
        .join(&args.target)
        .join("intervals.json");
    println!(
        "Seeded interval [{}, {}) for model '{}' on target '{}' -> {}",
        args.start,
        args.end,
        model.canonical_path(),
        args.target,
        intervals_path.display()
    );

    Ok(())
}
