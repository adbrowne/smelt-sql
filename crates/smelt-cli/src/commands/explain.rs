use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_selector_args},
    build_explain_output, discover_emitted_model_files, discover_python_models, find_project_root,
    init_db, parse_selector, Config, ModelDiscovery, SourcesConfig,
};
use smelt_core::graph::DependencyGraph;
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
    let mut models: Vec<smelt_cli::ModelFile> = sql_models
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

    let mut graph = DependencyGraph::build(models, sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    graph.add_seeds(&seeds);

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
            .select_models(&selectors, &config)
            .with_context(|| "Failed to select models")?;
        graph
            .filtered_execution_order(&selected)
            .with_context(|| "Failed to determine execution order")?
    };

    // Function bodies so batch-safety classification sees lookback declared
    // inside `smelt.define` bodies (parity with the execution path).
    let fn_bodies = smelt_runtime::build_fn_body_map(&db, ws);
    let mut output = build_explain_output(&graph, &config, &fn_bodies, &origins)?;
    // Narrow the output's execution_order to the filtered set so the
    // human-readable and JSON output reflects --select.
    output.execution_order = execution_order.clone();
    // Also filter the models map to only the selected set.
    output
        .models
        .retain(|name, _| execution_order.contains(name));

    // Build physical section via planner (no backends needed for explain).
    let mut opt_graph = ModelGraph::new();
    for model_name in &execution_order {
        let Ok(model) = graph.get_model(model_name) else {
            continue;
        };
        let metadata = model.metadata.as_deref();
        let frontmatter = Frontmatter::parse(&model.content);
        let inc_config = config
            .get_incremental_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| frontmatter.as_ref().and_then(|f| f.incremental.clone()));
        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));
        opt_graph.add_model(ModelInfo {
            name: model.name.clone(),
            sql: model.content.clone(),
            refs: model
                .refs
                .iter()
                .map(|r| r.smelt_ref.to_path().join("."))
                .collect(),
            timeseries_config: ts_config,
            incremental_config: inc_config,
        });
    }

    let planner = Planner::new();
    let (transformations, _plan_errors) = planner.plan(&opt_graph);

    // Build a PlanSummary-like physical section from planner results.
    // We don't call execute_project here since explain runs without backends.
    let physical = build_physical_section(&execution_order, &graph, &config, &transformations);
    output.physical = Some(physical);

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

/// Build the physical explain section from planner transformations and the
/// dependency graph (for explain mode — no backends needed).
fn build_physical_section(
    execution_order: &[String],
    graph: &DependencyGraph,
    config: &Config,
    transformations: &[smelt_planner::Transformation],
) -> smelt_cli::explain::ExplainPhysical {
    use smelt_cli::explain::{ExplainPhysical, ExplainPhysicalNode};
    use smelt_core::config::Materialization;
    use smelt_planner::Transformation;
    use std::collections::BTreeMap;

    let default_target = config.targets.keys().next().cloned().unwrap_or_default();

    // Parse transformations into lookup maps for physical section rendering.
    let mut incremental_overrides: HashMap<String, (String, String)> = HashMap::new();
    let mut planner_transformations: Vec<String> = Vec::new();

    for t in transformations {
        match t {
            Transformation::SetIncremental {
                model,
                partition_column,
                ..
            } => {
                incremental_overrides
                    .insert(model.clone(), (partition_column.clone(), "day".to_string()));
                planner_transformations.push(format!(
                    "{} → incremental (partition: {})",
                    model, partition_column
                ));
            }
            Transformation::ReplaceWithPlan { model, steps } => {
                planner_transformations.push(format!(
                    "{} → cube split ({} steps)",
                    model,
                    steps.len()
                ));
            }
            _ => {}
        }
    }

    let mut nodes = BTreeMap::new();
    let mut ephemerals = Vec::new();
    let mut phys_execution_order = Vec::new();

    for model_name in execution_order {
        let Ok(model_file) = graph.get_model(model_name) else {
            continue;
        };
        let metadata = model_file.metadata.as_deref();
        let materialization = config.get_materialization_with_metadata(model_name, metadata);
        let frontmatter = smelt_planner::Frontmatter::parse(&model_file.content);

        if materialization == Materialization::Ephemeral {
            ephemerals.push(model_name.clone());
            continue;
        }

        phys_execution_order.push(model_name.clone());

        let inc_config = config
            .get_incremental_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| frontmatter.as_ref().and_then(|f| f.incremental.clone()));
        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));

        let strategy = if let Some((part_col, gran)) = incremental_overrides.get(model_name) {
            format!(
                "incremental (partition: {}, granularity: {})",
                part_col, gran
            )
        } else if let (Some(_), Some(ts)) = (&inc_config, &ts_config) {
            let gran = serde_json::to_value(&ts.granularity)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "?".to_string());
            format!(
                "incremental (partition: {}, granularity: {})",
                ts.partition_column, gran
            )
        } else if metadata.is_some_and(|m| m.is_cumulative())
            || materialization == Materialization::CumulativeAggregate
        {
            "cumulative_aggregate".to_string()
        } else {
            "full_refresh".to_string()
        };

        let model_target = config.get_target(model_name, metadata, &default_target);

        nodes.insert(
            model_name.clone(),
            ExplainPhysicalNode {
                strategy,
                materialization,
                target: model_target,
                logical_origins: vec![model_name.clone()],
            },
        );
    }

    ExplainPhysical {
        execution_order: phys_execution_order,
        nodes,
        ephemerals,
        transformations: planner_transformations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::{
        config::{Materialization, RefreshStrategy},
        discovery::ModelKind,
        graph::DependencyGraph,
        metadata::ModelMetadata,
        model_id::ModelId,
        ModelFile,
    };

    /// Regression test: a model declared with `materialization: table` +
    /// `refresh: cumulative` must report `"cumulative_aggregate"` as its
    /// physical strategy in `smelt explain` output, NOT `"full_refresh"`.
    ///
    /// Before the fix, `build_physical_section` only checked for the legacy
    /// `Materialization::CumulativeAggregate` variant; the new-surface
    /// `Table` + `refresh: cumulative` combination fell through to
    /// `"full_refresh"`.
    #[test]
    fn refresh_cumulative_table_strategy_is_cumulative_aggregate() {
        // Minimal smelt.yml in a temp dir.
        let tmp = tempfile::TempDir::new().expect("create tempdir");
        let yml = "name: test_proj\n\
                   version: 1\n\
                   paths:\n  - models\n\
                   targets:\n  dev:\n    type: duckdb\n    schema: main\n\
                   default_materialization: view\n";
        std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();
        let config = smelt_cli::Config::load(tmp.path()).expect("Config::load from temp smelt.yml");

        // Build a ModelFile with `materialization: table` + `refresh: cumulative`.
        let model_name = "my_cumulative_model";
        let path: std::path::PathBuf = format!("models/{}.sql", model_name).into();
        let metadata = ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Cumulative),
            ..ModelMetadata::default()
        };
        let model_file = ModelFile {
            name: model_name.to_string(),
            model_id: ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs: vec![],
            parse_errors: vec![],
            metadata: Some(Box::new(metadata)),
            kind: ModelKind::Sql,
            address_segments: vec![model_name.to_string()],
        };

        let graph = DependencyGraph::build(vec![model_file], None).expect("DependencyGraph::build");

        let execution_order = vec![model_name.to_string()];
        let physical = build_physical_section(&execution_order, &graph, &config, &[]);

        let node = physical.nodes.get(model_name).unwrap_or_else(|| {
            panic!(
                "expected node '{}' in physical section; got: {:?}",
                model_name,
                physical.nodes.keys().collect::<Vec<_>>()
            )
        });

        assert_eq!(
            node.strategy, "cumulative_aggregate",
            "model with materialization: table + refresh: cumulative must report \
             strategy 'cumulative_aggregate', not '{}'",
            node.strategy
        );
    }

    /// Sanity check: a plain `materialization: table` model (no refresh: cumulative)
    /// still reports `"full_refresh"`.
    #[test]
    fn plain_table_strategy_is_full_refresh() {
        let tmp = tempfile::TempDir::new().expect("create tempdir");
        let yml = "name: test_proj\n\
                   version: 1\n\
                   paths:\n  - models\n\
                   targets:\n  dev:\n    type: duckdb\n    schema: main\n\
                   default_materialization: view\n";
        std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();
        let config = smelt_cli::Config::load(tmp.path()).expect("Config::load from temp smelt.yml");

        let model_name = "plain_table";
        let path: std::path::PathBuf = format!("models/{}.sql", model_name).into();
        let metadata = ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: None,
            ..ModelMetadata::default()
        };
        let model_file = ModelFile {
            name: model_name.to_string(),
            model_id: ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs: vec![],
            parse_errors: vec![],
            metadata: Some(Box::new(metadata)),
            kind: ModelKind::Sql,
            address_segments: vec![model_name.to_string()],
        };

        let graph = DependencyGraph::build(vec![model_file], None).expect("DependencyGraph::build");
        let execution_order = vec![model_name.to_string()];
        let physical = build_physical_section(&execution_order, &graph, &config, &[]);

        let node = physical.nodes.get(model_name).expect("node exists");
        assert_eq!(
            node.strategy, "full_refresh",
            "plain table model must report strategy 'full_refresh'"
        );
    }
}
