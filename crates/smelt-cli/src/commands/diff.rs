use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_selector_args},
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

pub async fn diff(args: DiffArgs, scope: Option<&str>) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

    // Seeds participate as `smelt.ref()` targets (bug #2 in 20260417 follow-up).
    let seeds = smelt_core::discover_seed_infos(&project_dir, &config.paths);

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
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

    // Initialise the Salsa DB early so scope resolution can call
    // smelt_db::resolve_ref_path / leaf_did_you_mean.
    let db = init_db(&project_dir, &models);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let project = db
        .project_input(&project_dir)
        .expect("project not initialized");

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

    // Resolve --select / --exclude args through scope before parsing selectors.
    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
    let resolved_select =
        resolve_selector_args(&db, ws, project, active_scope.as_ref(), &args.select)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    let resolved_exclude =
        resolve_selector_args(&db, ws, project, active_scope.as_ref(), &args.exclude)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Apply --select / --exclude filters
    let has_selectors = !resolved_select.is_empty() || !resolved_exclude.is_empty();
    let selected_names: HashSet<String> = if has_selectors {
        let mut selected = if !resolved_select.is_empty() {
            let selectors: Vec<_> = resolved_select
                .iter()
                .map(|s| parse_selector(s).with_context(|| format!("Invalid selector '{}'", s)))
                .collect::<Result<_, _>>()?;
            graph.select_models(&selectors)?
        } else {
            graph.execution_order()?.into_iter().collect::<HashSet<_>>()
        };

        if !resolved_exclude.is_empty() {
            let excludes: Vec<_> = resolved_exclude
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

    let file_store = FileStore::new(&project_dir);

    // Build model name → ModelFile lookup keyed by the canonical dot-path
    // (`graph.execution_order()` / `ordered_models` use canonical keys). Keying
    // by the leaf `m.name` would miss sub-directory models (canonical
    // `staging.stg_orders` ≠ leaf `stg_orders`), silently skipping them.
    let all_models: Vec<_> = graph.iter_models().map(|(_, m)| m.clone()).collect();
    let model_lookup: std::collections::HashMap<String, &smelt_cli::ModelFile> =
        all_models.iter().map(|m| (m.canonical_path(), m)).collect();

    // Diff each selected model
    let mut entries: Vec<ModelDiffEntry> = Vec::new();

    for name in &ordered_models {
        let Some(model) = model_lookup.get(name.as_str()) else {
            continue;
        };

        let inferred = infer_deployed_columns(&db, model);

        // Stored schemas are keyed by the db-name (`db_name_owned()`), matching
        // the run-pipeline save path. Loading by the canonical `name` would
        // never find a sub-directory model's schema (always reporting it NEW).
        let db_name = model.db_name_owned();
        let deployed = file_store
            .load_schema(&db_name)
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
                        &db_name,
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
        // Deployed schema files are keyed by db-name; compare against db-names
        // so a live sub-directory model is not falsely reported REMOVED.
        let discovered_names: HashSet<String> =
            all_models.iter().map(|m| m.db_name_owned()).collect();
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
                for warning in &diff.warnings {
                    println!("  ⚠ {}", warning);
                }
                for line in diff.summary() {
                    println!("  {}", line);
                }
                match action {
                    MigrationAction::NoChange => {}
                    MigrationAction::AlterTable { statements } => {
                        println!("  -> Safe: ALTER TABLE (no data loss)");
                        for stmt in statements {
                            println!("     {}", stmt);
                        }
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
                    MigrationAction::TableRewrite { select_expr } => {
                        println!("  -> Requires: table rewrite ({})", select_expr);
                    }
                    MigrationAction::FullRefreshBlocked { reason } => {
                        println!("  -> Blocked: requires --allow-full-refresh ({})", reason);
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
                    MigrationAction::TableRewrite { .. } => ("table_rewrite", vec![]),
                    MigrationAction::FullRefreshBlocked { .. } => ("full_refresh_blocked", vec![]),
                };

                Some(json!({
                    "name": entry.name,
                    "status": "changed",
                    "changes": changes,
                    "warnings": diff.warnings,
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
