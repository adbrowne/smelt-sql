use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_selector_args},
    build_explain_output, build_physical_explain, discover_emitted_model_files,
    discover_python_models, find_project_root, init_db, parse_selector, Config, LogicalGraph,
    ModelDiscovery, ModelFile, PhysicalGraphBuilder, SourcesConfig,
};
use smelt_planner::{Frontmatter, ModelGraph, ModelInfo, Planner};
use std::collections::HashMap;

use crate::ExplainArgs;

pub async fn explain(args: ExplainArgs, scope: Option<&str>) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

    // Seeds are valid `smelt.ref()` targets and should appear in the graph.
    let seeds = smelt_core::discover_seed_infos(&project_dir, &config.paths);

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let sql_models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Build Salsa DB from all raw SQL files (including generator files) so
    // the emitted-models pipeline can run via `smelt_db::emitted_models()`,
    // and so scope resolution can call smelt_db::resolve_ref_path /
    // leaf_did_you_mean.
    let db = init_db(&project_dir, &sql_models);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let project = db
        .project_input(&project_dir)
        .expect("project not initialized");

    // Discover generator-emitted models and their provenance.
    let (emitted_model_files, origins) =
        discover_emitted_model_files(&db, &project_dir, &config.paths);

    // Build the model list:
    //   - Exclude generator files (.gen.sql) from the hand-authored set so they
    //     don't appear as both a generator and a regular model.
    //   - Include the emitted virtual ModelFile entries produced above.
    let mut models: Vec<ModelFile> = sql_models
        .into_iter()
        .filter(|m| !m.name.ends_with(".gen") && !m.path.to_string_lossy().contains(".gen."))
        .collect();
    models.extend(emitted_model_files);

    // Filter out test models — they shouldn't appear in explain output
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

    // Explain doesn't execute, so use the first target as default
    let default_target = config
        .targets
        .keys()
        .next()
        .map(|s| s.as_str())
        .unwrap_or("dev");
    let mut graph = LogicalGraph::build(models, sources.as_ref(), &seeds, &config, default_target)
        .with_context(|| "Failed to build logical graph")?;

    // Annotate generator-emitted nodes with their provenance.
    graph.annotate_emitted_models(&origins);

    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    // Apply --select filtering if provided.  Must happen before
    // build_explain_output so the output reflects the filtered model set.
    let execution_order: Vec<String> = if args.select.is_empty() {
        graph.execution_order()?
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
        let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
        let resolved_select =
            resolve_selector_args(&db, ws, project, active_scope.as_ref(), &args.select)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
        let selectors: Vec<_> = resolved_select
            .iter()
            .map(|s| parse_selector(s).with_context(|| format!("Invalid selector '{}'", s)))
            .collect::<Result<_, _>>()?;
        let selected = graph
            .select_models(&selectors)
            .with_context(|| "Failed to select models")?;
        graph
            .filtered_execution_order(&selected)
            .with_context(|| "Failed to determine execution order")?
    };

    // Function bodies so batch-safety classification sees lookback declared
    // inside `smelt.define` bodies (parity with the execution path).
    let fn_bodies = smelt_runtime::build_fn_body_map(&db, ws);
    let mut output = build_explain_output(&graph, &fn_bodies)?;
    // Narrow the output's execution_order to the filtered set so the
    // human-readable and JSON output reflects --select.
    output.execution_order = execution_order.clone();
    // Also filter the models map to only the selected set.
    output
        .models
        .retain(|name, _| execution_order.contains(name));

    // Build physical graph via planner (no backends needed for explain)
    let mut opt_graph = ModelGraph::new();
    for model_name in &execution_order {
        let model = graph.get_model(model_name)?;
        let frontmatter = Frontmatter::parse(&model.content);
        opt_graph.add_model(ModelInfo {
            name: model.name.clone(),
            sql: model.content.clone(),
            refs: model
                .refs
                .iter()
                .map(|r| r.smelt_ref.to_path().join("."))
                .collect(),
            timeseries_config: frontmatter.as_ref().and_then(|f| f.timeseries.clone()),
            incremental_config: frontmatter.as_ref().and_then(|f| f.incremental.clone()),
        });
    }

    let planner = Planner::new();
    let (transformations, _plan_errors) = planner.plan(&opt_graph);

    let target_schemas: HashMap<String, String> = config
        .targets
        .iter()
        .map(|(k, v)| (k.clone(), v.schema.clone()))
        .collect();

    let physical_graph =
        PhysicalGraphBuilder::for_explain(&graph, &transformations, target_schemas)
            .build()
            .with_context(|| "Failed to build physical graph for explain")?;

    output.physical = Some(build_physical_explain(&physical_graph, &graph));

    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        // Human-readable output
        println!("Project: {} (version {})", config.name, config.version);
        println!(
            "Models: {} | Execution order: {}",
            output.models.len(),
            output.execution_order.join(" → ")
        );
        println!();

        println!("Logical Graph:");
        for name in &output.execution_order {
            if let Some(model) = output.models.get(name) {
                let mat = serde_json::to_value(&model.materialization)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "unknown".to_string());

                print!("  {} [{}]", name, mat);

                if !model.dependencies.is_empty() {
                    print!(" ← {}", model.dependencies.join(", "));
                }

                if let Some(ref inc) = model.incremental {
                    print!(
                        " (incremental: {} by {}, {})",
                        serde_json::to_value(&inc.granularity)
                            .ok()
                            .and_then(|v| v.as_str().map(|s| s.to_string()))
                            .unwrap_or_else(|| "?".to_string()),
                        inc.partition_column,
                        inc.batch_safety
                    );
                }

                if !model.tags.is_empty() {
                    print!(" [{}]", model.tags.join(", "));
                }

                if let Some(ref owner) = model.owner {
                    print!(" @{}", owner);
                }

                println!();
            }
        }

        if let Some(ref phys) = output.physical {
            println!("\nPhysical Graph:");
            if !phys.ephemerals.is_empty() {
                println!(
                    "  Ephemeral (inlined as CTEs): {}",
                    phys.ephemerals.join(", ")
                );
            }
            if !phys.transformations.is_empty() {
                println!("  Planner optimizations:");
                for t in &phys.transformations {
                    println!("    {}", t);
                }
            }
            println!("  Execution order: {}", phys.execution_order.join(" → "));
            for name in &phys.execution_order {
                if let Some(node) = phys.nodes.get(name) {
                    let mat = serde_json::to_value(&node.materialization)
                        .ok()
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| "unknown".to_string());
                    print!("  {} [{}] {}", name, mat, node.strategy);
                    if node.logical_origins.len() > 1
                        || (node.logical_origins.len() == 1 && node.logical_origins[0] != *name)
                    {
                        print!(" (from: {})", node.logical_origins.join(", "));
                    }
                    println!();
                }
            }
        }

        println!("\nTip: Use --json for machine-readable output.");
    }

    Ok(())
}
