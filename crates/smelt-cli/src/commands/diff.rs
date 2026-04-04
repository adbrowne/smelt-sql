use anyhow::{Context, Result};
use smelt_cli::{
    discover_python_models, find_project_root, init_db, migration, parse_selector, Config,
    LogicalGraph, ModelDiscovery, SourcesConfig,
};
use smelt_state::file_store::FileStore;
use smelt_state::schema_tracking::{diff_schemas, plan_migration, MigrationAction, SchemaDiff};
use std::collections::HashSet;

use crate::helpers::infer_deployed_columns;
use crate::DiffArgs;

enum ModelDiffStatus {
    New,
    Unchanged,
    Changed {
        diff: SchemaDiff,
        action: MigrationAction,
    },
    Removed,
}

struct ModelDiffEntry {
    name: String,
    status: ModelDiffStatus,
}

pub async fn diff(args: DiffArgs) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

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
    let graph = LogicalGraph::build(models.clone(), sources.as_ref(), &config, default_target)
        .with_context(|| "Failed to build logical graph")?;

    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    // Apply --select / --exclude filters
    let has_selectors = !args.select.is_empty() || !args.exclude.is_empty();
    let selected_names: HashSet<String> = if has_selectors {
        let mut selected = if !args.select.is_empty() {
            let selectors: Vec<_> = args
                .select
                .iter()
                .map(|s| parse_selector(s).with_context(|| format!("Invalid selector '{}'", s)))
                .collect::<Result<_, _>>()?;
            graph.select_models(&selectors)?
        } else {
            graph.execution_order()?.into_iter().collect::<HashSet<_>>()
        };

        if !args.exclude.is_empty() {
            let excludes: Vec<_> = args
                .exclude
                .iter()
                .map(|s| {
                    parse_selector(s).with_context(|| format!("Invalid exclude selector '{}'", s))
                })
                .collect::<Result<_, _>>()?;
            selected = graph.exclude_models(&selected, &excludes)?;
        }

        selected
    } else {
        graph.execution_order()?.into_iter().collect::<HashSet<_>>()
    };

    // Get models in execution order for consistent output
    let exec_order = graph.execution_order()?;
    let ordered_models: Vec<_> = exec_order
        .iter()
        .filter(|name| selected_names.contains(name.as_str()))
        .collect();

    // Initialize Salsa DB for type inference
    let all_models: Vec<_> = graph.iter_models().map(|(_, m)| m.clone()).collect();
    let db = init_db(&project_dir, &all_models);

    let file_store = FileStore::new(&project_dir);

    // Build model name → ModelFile lookup
    let model_lookup: std::collections::HashMap<&str, &smelt_cli::ModelFile> =
        all_models.iter().map(|m| (m.name.as_str(), m)).collect();

    // Diff each selected model
    let mut entries: Vec<ModelDiffEntry> = Vec::new();

    for name in &ordered_models {
        let Some(model) = model_lookup.get(name.as_str()) else {
            continue;
        };

        let inferred = infer_deployed_columns(&db, model);

        let deployed = file_store
            .load_schema(name)
            .with_context(|| format!("Failed to load deployed schema for {}", name))?;

        let status = match deployed {
            None => ModelDiffStatus::New,
            Some(deployed_schema) => {
                let schema_diff = diff_schemas(&deployed_schema.columns, &inferred);
                if schema_diff.is_empty() {
                    ModelDiffStatus::Unchanged
                } else {
                    // Use the model's target schema for ALTER TABLE statement generation
                    let schema_name = graph
                        .get_node(name)
                        .ok()
                        .and_then(|node| config.targets.get(&node.target))
                        .map(|t| t.schema.as_str())
                        .unwrap_or("main");
                    let (column_defaults, backfill_exprs) =
                        migration::extract_evolution_maps(model.metadata.as_deref());

                    let action = plan_migration(
                        schema_name,
                        name,
                        &schema_diff,
                        true, // show full plan, don't block on column removal
                        &column_defaults,
                        &backfill_exprs,
                    );
                    ModelDiffStatus::Changed {
                        diff: schema_diff,
                        action,
                    }
                }
            }
        };

        entries.push(ModelDiffEntry {
            name: name.to_string(),
            status,
        });
    }

    // Detect removed models (only when no selectors active)
    if !has_selectors {
        let discovered_names: HashSet<&str> = all_models.iter().map(|m| m.name.as_str()).collect();
        let deployed_names = file_store.list_deployed_model_names();
        for deployed_name in &deployed_names {
            if !discovered_names.contains(deployed_name.as_str()) {
                entries.push(ModelDiffEntry {
                    name: deployed_name.clone(),
                    status: ModelDiffStatus::Removed,
                });
            }
        }
    }

    // Output
    if args.json {
        print_json(&entries);
    } else {
        print_text(&entries);
    }

    // Exit with code 1 if there are any changes (useful for CI)
    let has_changes = entries
        .iter()
        .any(|e| !matches!(e.status, ModelDiffStatus::Unchanged));
    if has_changes {
        std::process::exit(1);
    }

    Ok(())
}

fn print_text(entries: &[ModelDiffEntry]) {
    let mut changed = 0u32;
    let mut new = 0u32;
    let mut removed = 0u32;
    let mut unchanged = 0u32;

    for entry in entries {
        match &entry.status {
            ModelDiffStatus::Unchanged => {
                unchanged += 1;
            }
            ModelDiffStatus::New => {
                new += 1;
                println!("Model: {}", entry.name);
                println!("  + New model (not yet deployed)");
                println!();
            }
            ModelDiffStatus::Removed => {
                removed += 1;
                println!("Model: {}", entry.name);
                println!("  - Removed (deployed schema exists but model not found in code)");
                println!();
            }
            ModelDiffStatus::Changed { diff, action } => {
                changed += 1;
                println!("Model: {}", entry.name);
                for line in diff.summary() {
                    println!("  {}", line);
                }
                match action {
                    MigrationAction::NoChange => {}
                    MigrationAction::AlterTable { .. } => {
                        println!("  -> Safe: ALTER TABLE (no data loss)");
                    }
                    MigrationAction::FullRefresh { reason } => {
                        println!("  -> Requires: --full-refresh ({})", reason);
                    }
                    MigrationAction::RequiresColumnRemovalFlag { columns } => {
                        println!(
                            "  -> Requires: --allow-column-removal ({} column{})",
                            columns.len(),
                            if columns.len() == 1 { "" } else { "s" }
                        );
                    }
                }
                println!();
            }
        }
    }

    let total = changed + new + removed + unchanged;
    if changed == 0 && new == 0 && removed == 0 {
        println!(
            "No schema changes detected ({} model{} checked)",
            total,
            if total == 1 { "" } else { "s" }
        );
    } else {
        println!(
            "Summary: {} changed, {} new, {} removed, {} unchanged",
            changed, new, removed, unchanged
        );
    }
}

fn print_json(entries: &[ModelDiffEntry]) {
    use serde_json::json;

    let mut changed = 0u32;
    let mut new = 0u32;
    let mut removed = 0u32;
    let mut unchanged = 0u32;

    let models: Vec<_> = entries
        .iter()
        .filter_map(|entry| match &entry.status {
            ModelDiffStatus::Unchanged => {
                unchanged += 1;
                None
            }
            ModelDiffStatus::New => {
                new += 1;
                Some(json!({
                    "name": entry.name,
                    "status": "new"
                }))
            }
            ModelDiffStatus::Removed => {
                removed += 1;
                Some(json!({
                    "name": entry.name,
                    "status": "removed"
                }))
            }
            ModelDiffStatus::Changed { diff, action } => {
                changed += 1;
                let changes: Vec<_> =
                    diff.changes
                        .iter()
                        .map(|c| {
                            match c {
                        smelt_state::schema_tracking::SchemaChange::AddColumn {
                            name,
                            data_type,
                            nullable,
                        } => json!({
                            "type": "add_column",
                            "column": name,
                            "data_type": data_type,
                            "nullable": nullable
                        }),
                        smelt_state::schema_tracking::SchemaChange::RemoveColumn { name } => {
                            json!({
                                "type": "remove_column",
                                "column": name
                            })
                        }
                        smelt_state::schema_tracking::SchemaChange::ChangeType {
                            name,
                            from,
                            to,
                        } => json!({
                            "type": "change_type",
                            "column": name,
                            "from": from,
                            "to": to
                        }),
                        smelt_state::schema_tracking::SchemaChange::ChangeNullability {
                            name,
                            from_nullable,
                            to_nullable,
                        } => json!({
                            "type": "change_nullability",
                            "column": name,
                            "from_nullable": from_nullable,
                            "to_nullable": to_nullable
                        }),
                        smelt_state::schema_tracking::SchemaChange::StructFieldAdded {
                            column, path, field_name, field_type, nullable,
                        } => json!({
                            "type": "struct_field_added",
                            "column": column,
                            "path": path,
                            "field_name": field_name,
                            "field_type": field_type,
                            "nullable": nullable
                        }),
                        smelt_state::schema_tracking::SchemaChange::StructFieldRemoved {
                            column, path, field_name,
                        } => json!({
                            "type": "struct_field_removed",
                            "column": column,
                            "path": path,
                            "field_name": field_name
                        }),
                        smelt_state::schema_tracking::SchemaChange::NestedTypeChange {
                            column, path, from, to,
                        } => json!({
                            "type": "nested_type_change",
                            "column": column,
                            "path": path,
                            "from": from,
                            "to": to
                        }),
                        smelt_state::schema_tracking::SchemaChange::ArrayElementTypeChange {
                            column, path, from, to,
                        } => json!({
                            "type": "array_element_type_change",
                            "column": column,
                            "path": path,
                            "from": from,
                            "to": to
                        }),
                        smelt_state::schema_tracking::SchemaChange::MapKeyTypeChange {
                            column, from, to,
                        } => json!({
                            "type": "map_key_type_change",
                            "column": column,
                            "from": from,
                            "to": to
                        }),
                        smelt_state::schema_tracking::SchemaChange::MapValueTypeChange {
                            column, path, from, to,
                        } => json!({
                            "type": "map_value_type_change",
                            "column": column,
                            "path": path,
                            "from": from,
                            "to": to
                        }),
                        smelt_state::schema_tracking::SchemaChange::IncompatibleTypeChange {
                            column, from, to, reason,
                        } => json!({
                            "type": "incompatible_type_change",
                            "column": column,
                            "from": from,
                            "to": to,
                            "reason": reason
                        }),
                    }
                        })
                        .collect();

                let (action_str, statements) = match action {
                    MigrationAction::NoChange => ("no_change", vec![]),
                    MigrationAction::AlterTable { statements } => {
                        ("alter_table", statements.clone())
                    }
                    MigrationAction::FullRefresh { .. } => ("full_refresh", vec![]),
                    MigrationAction::RequiresColumnRemovalFlag { .. } => {
                        ("requires_column_removal_flag", vec![])
                    }
                };

                Some(json!({
                    "name": entry.name,
                    "status": "changed",
                    "changes": changes,
                    "risk": {
                        "requires_full_refresh": diff.requires_full_refresh(),
                        "has_column_removals": diff.has_column_removals(),
                        "migration_action": action_str,
                        "statements": statements
                    }
                }))
            }
        })
        .collect();

    let output = json!({
        "models": models,
        "summary": {
            "changed": changed,
            "new": new,
            "removed": removed,
            "unchanged": unchanged
        }
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("JSON serialization should not fail")
    );
}
