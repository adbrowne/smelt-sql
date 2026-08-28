//! `smelt migrate <model>` — plan-only definition-delta migration verb.
//!
//! Derives a [`smelt_logical::backbuild::DefinitionDiff`] between the model's
//! last-deployed SQL (`DeployedSchema::model_sql`) and its current SQL,
//! classifies it via `smelt_logical::backbuild`, and prints a per-column-group
//! migration plan (verdict + technique + plan hash). Executes nothing — no
//! backend connection is opened beyond the schema-snapshot file read.
//! `--apply` is not yet a flag (a later phase); see
//! `docs/specs/definition_deltas.md` §"`smelt migrate`".

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};

use smelt_cli::{find_project_root, Config, ModelDiscovery, SourcesConfig};
use smelt_logical::backbuild::{
    definition_diff, derive_migration_plan, plan_hash, BackbuildInputs, ColumnGroupPlan,
    MigrationPlan, MigrationVerdict, SourceRef,
};
use smelt_state::file_store::FileStore;

use crate::helpers::infer_deployed_columns;
use crate::MigrateArgs;

pub async fn migrate(args: MigrateArgs, _scope: Option<&str>) -> Result<()> {
    // 1. Resolve project root + config + target.
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;
    if !config.targets.contains_key(&args.target) {
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
    }

    // 2. Load legacy sources.yml (best-effort; no unique_key/nullability
    // facts exist in this format — populated fail-closed below).
    let sources = SourcesConfig::load(&project_dir).ok();

    // 3. Discover models and find the target model by name/canonical path.
    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    let model = models
        .iter()
        .find(|m| m.canonical_path() == args.model || m.name == args.model)
        .ok_or_else(|| anyhow::anyhow!("Model '{}' not found", args.model))?;

    let model_name = model.canonical_path();
    let db_name = model.db_name_owned();

    // 4. Load the recorded (last-deployed) schema for this model+target.
    let file_store = FileStore::new(&project_dir, &args.target);
    let deployed = file_store
        .load_schema(&db_name)
        .with_context(|| format!("Failed to load deployed schema for {}", model_name))?;

    let Some(deployed) = deployed else {
        anyhow::bail!(
            "smelt migrate: no recorded definition for '{model_name}' — the model has never \
             been deployed under schema tracking on target '{}'. Run `smelt run` first.",
            args.target
        );
    };

    let Some(before_sql_raw) = deployed.model_sql.clone() else {
        anyhow::bail!(
            "smelt migrate: the recorded schema for '{model_name}' on target '{}' predates \
             definition-SQL tracking (no `model_sql` recorded) — there is no definition to \
             diff against. Run `smelt run` again to record one.",
            args.target
        );
    };

    // 5. Parse both sides and derive the definition diff.
    let before_clean = smelt_parser::strip_frontmatter(&before_sql_raw);
    let before_parse = smelt_parser::parse(&before_clean);
    let before_file = smelt_parser::File::cast(before_parse.syntax()).ok_or_else(|| {
        anyhow::anyhow!("Failed to parse the recorded definition for '{model_name}'")
    })?;

    let after_clean = smelt_parser::strip_frontmatter(&model.content);
    let after_parse = smelt_parser::parse(&after_clean);
    let after_file = smelt_parser::File::cast(after_parse.syntax()).ok_or_else(|| {
        anyhow::anyhow!("Failed to parse the current definition for '{model_name}'")
    })?;

    let diff = definition_diff(&before_file, &after_file);

    // 6. Build BackbuildInputs.
    let db = smelt_cli::init_db(&project_dir, &models);
    let inferred = infer_deployed_columns(&db, model);

    let row_identity = model.metadata.as_ref().and_then(|m| m.unique_key.clone());

    let not_null_columns: BTreeSet<String> = deployed
        .columns
        .iter()
        .filter(|c| !c.nullable)
        .map(|c| c.name.clone())
        .collect();

    let deployed_names: BTreeSet<&str> = deployed.columns.iter().map(|c| c.name.as_str()).collect();
    let added_column_types: BTreeMap<String, String> = inferred
        .iter()
        .filter(|c| !deployed_names.contains(c.name.as_str()))
        .map(|c| (c.name.clone(), c.data_type.clone()))
        .collect();

    // `sources` is deliberately best-effort: keyed by each ref's leaf name
    // (not proven alias-exact against the FROM-tree — a documented
    // simplification, see the module doc). A source or upstream model whose
    // facts cannot be found is simply left out — fail-closed, only ever
    // costing admitted techniques, never a wrong admission.
    let mut sources_map: BTreeMap<String, SourceRef> = BTreeMap::new();
    for r in &model.refs {
        let path = r.smelt_ref.to_path();
        let leaf = r.smelt_ref.leaf_name();
        if leaf.is_empty() || sources_map.contains_key(&leaf) {
            continue;
        }

        // Legacy sources.yml: `smelt.sources.<source>.<table>`.
        if path.first().map(String::as_str) == Some("sources") && path.len() >= 3 {
            let source_name = &path[path.len() - 2];
            let table_name = &path[path.len() - 1];
            if let Some(source_def) = sources
                .as_ref()
                .and_then(|sc| sc.sources.iter().find(|s| &s.name == source_name))
            {
                if let Some(table_def) = source_def.tables.iter().find(|t| &t.name == table_name) {
                    sources_map.insert(
                        leaf.clone(),
                        SourceRef {
                            physical_name: table_def
                                .identifier
                                .clone()
                                .unwrap_or_else(|| table_def.name.clone()),
                            unique_key: None,
                            not_null_columns: BTreeSet::new(),
                        },
                    );
                    continue;
                }
            }
        }

        // Otherwise, try resolving against another discovered model.
        let canonical = path.join(".");
        if let Some(upstream) = models
            .iter()
            .find(|m| m.canonical_path() == canonical || m.name == leaf)
        {
            let upstream_unique_key = upstream
                .metadata
                .as_ref()
                .and_then(|m| m.unique_key.clone());
            let upstream_not_null: BTreeSet<String> = file_store
                .load_schema(&upstream.db_name_owned())
                .ok()
                .flatten()
                .map(|s| {
                    s.columns
                        .iter()
                        .filter(|c| !c.nullable)
                        .map(|c| c.name.clone())
                        .collect()
                })
                .unwrap_or_default();
            sources_map.insert(
                leaf,
                SourceRef {
                    physical_name: upstream.db_name_owned(),
                    unique_key: upstream_unique_key,
                    not_null_columns: upstream_not_null,
                },
            );
        }
    }

    let inputs = BackbuildInputs {
        table: db_name.clone(),
        after_sql: after_clean.clone(),
        row_identity,
        not_null_columns,
        added_column_types,
        sources: sources_map,
    };

    // 7. Derive and render the plan. No execution happens anywhere below.
    let plan = derive_migration_plan(&model_name, &diff, &inputs);
    let hash = plan_hash(&plan, &inputs);

    render_plan(&model_name, &plan, &hash);

    Ok(())
}

fn render_plan(model_name: &str, plan: &MigrationPlan, hash: &str) {
    if plan.groups.is_empty() {
        println!("definition delta for {model_name}: eclipsed — nothing to do");
        return;
    }

    println!(
        "definition delta for {model_name} ({} column group{} affected):",
        plan.groups.len(),
        if plan.groups.len() == 1 { "" } else { "s" }
    );
    println!();

    for group in &plan.groups {
        render_group(group);
    }

    println!("plan hash: {hash}   approve and execute with: smelt migrate {model_name} --apply");
}

fn render_group(group: &ColumnGroupPlan) {
    let label = if group.columns.is_empty() {
        "(skeleton)".to_string()
    } else {
        group.columns.join(", ")
    };

    let verdict_label = match group.verdict {
        MigrationVerdict::Eclipsed => "eclipsed",
        MigrationVerdict::BackfillInPlace => "backfill in place",
        MigrationVerdict::Rederive => "rederive",
        MigrationVerdict::SkeletonChange => "skeleton change (full refresh only)",
    };

    if let Some(option) = group.options.first() {
        println!(
            "  {label:<18}{verdict_label:<20} {:?} ({} statement{})",
            option.technique,
            option.statement_count(),
            if option.statement_count() == 1 {
                ""
            } else {
                "s"
            }
        );
    } else {
        println!("  {label:<18}{verdict_label:<20} no admissible technique");
    }

    for refusal in &group.refusals {
        println!("                    refused: {}", refusal.reason);
    }
    println!();
}
