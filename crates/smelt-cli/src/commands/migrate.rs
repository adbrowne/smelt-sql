use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use chrono::Utc;
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_argument},
    find_project_root, init_db, CliError, Config, ModelDiscovery, SourcesConfig,
};
use smelt_core::graph::DependencyGraph;
use smelt_logical::backbuild::{
    plan_hash, ColumnGroupPlan, CostClass, MigrationPlan, SourceRef, TechniqueCandidate, Verdict,
};
use smelt_runtime::migrate::{derive_migration_plan_for_model, MigrateError, ModelMigrationFacts};
use smelt_runtime::schema_evolution::infer_deployed_columns;
use smelt_state::file_store::FileStore;

use crate::MigrateArgs;

pub async fn migrate(args: MigrateArgs, scope: Option<&str>) -> Result<()> {
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

    let sources = SourcesConfig::load(&project_dir).ok();
    let seeds = smelt_core::discover_seed_infos(&project_dir, &config.paths);
    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;
    models.retain(|m| !m.is_assertion());

    let db = init_db(&project_dir, &models);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let project = db
        .project_input(&project_dir)
        .expect("project not initialized");

    let mut graph = DependencyGraph::build(models.clone(), sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    graph.add_seeds(&seeds);

    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
    let canonical = resolve_argument(&db, ws, project, active_scope.as_ref(), &args.model)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let model = models
        .iter()
        .find(|m| m.canonical_path() == canonical)
        .ok_or_else(|| anyhow::anyhow!("Model '{}' not found", canonical))?;

    // Diff against the model's raw SQL text, not compiled output —
    // `smelt_runtime::execute::execute_project` records `model.content`
    // (never a compiled/type-cast form) as `DeployedSchema::definition_sql`
    // (`crate::schema_evolution::save_deployed_schema`'s `model_sql` param
    // is always `plan.sql`, itself always `model.content.clone()` at every
    // `ModelPlan` construction site in `execute.rs`), so the "after" side
    // must be the same raw form or every unchanged model would spuriously
    // diff against its own type-cast-wrapped recompilation.
    let after_sql = model.content.clone();

    let db_name = model.db_name_owned();
    let file_store = FileStore::new(&project_dir, &args.target, config.state.mode);
    let deployed = file_store
        .load_schema(&db_name)
        .with_context(|| format!("Failed to load deployed schema for {}", canonical))?;

    let current_columns = infer_deployed_columns(&db, model);

    let row_identity = config
        .get_unique_key_with_metadata(&canonical, model.metadata.as_deref())
        .map(|k| k.to_vec());

    // One `SourceRef` per direct upstream model — a coarser approximation
    // than a full FROM-tree alias walk (phase 1 scope; see
    // `docs/outcomes/20260816-definition-delta-migrate-v2/phases/01-summary.md`
    // "For the next planner"). A self-join or a model referencing the same
    // upstream under two aliases is not yet distinguished.
    let mut source_facts: BTreeMap<String, SourceRef> = BTreeMap::new();
    for upstream_name in graph.get_upstream(&canonical) {
        let unique_key = config.get_unique_key(&upstream_name).map(|k| k.to_vec());
        let not_null_columns: BTreeSet<String> = file_store
            .load_schema(&upstream_name)
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
        source_facts.insert(
            upstream_name.clone(),
            SourceRef {
                physical_name: upstream_name,
                unique_key,
                not_null_columns,
            },
        );
    }

    let facts = ModelMigrationFacts {
        table: db_name,
        after_sql,
        row_identity,
        current_columns,
        deployed,
        sources: source_facts,
    };

    let (inputs, plan) = match derive_migration_plan_for_model(&canonical, &facts) {
        Ok(pair) => pair,
        Err(MigrateError::NoRecordedDefinition { model }) => {
            return Err(anyhow::anyhow!(
                "smelt migrate: model '{}' has no recorded definition — deploy it at least once \
                 (`smelt build` or `smelt run`) before running `smelt migrate`",
                model
            ));
        }
        Err(e) => return Err(anyhow::anyhow!("smelt migrate: {}", e)),
    };

    let hash = plan_hash(&inputs, &plan);
    let mut approvals = file_store
        .load_migration_approvals()
        .with_context(|| "Failed to load migration approval store")?;
    let previously_approved =
        approvals.get(&canonical).map(|a| a.plan_hash.as_str()) == Some(hash.as_str());

    // Eclipsed: nothing to approve or apply either way (§Surface "Plan-and-
    // approve": "An eclipsed-only delta is the one case with nothing to
    // approve; it completes on the next plan step.").
    if plan.eclipsed {
        render(&canonical, &plan, &hash, previously_approved, args.json);
        return Ok(());
    }

    if args.apply {
        if previously_approved {
            render(&canonical, &plan, &hash, previously_approved, args.json);
            if !args.json {
                println!(
                    "\napproved — nothing to execute yet (execution lands with the apply path)"
                );
            }
            return Ok(());
        }

        // Absent or stale: re-record the freshly derived plan and refuse to
        // execute, printing the plan the operator must re-approve
        // (`docs/specs/definition_deltas.md` §Surface "Approve and apply").
        approvals.record(&canonical, hash.clone(), Utc::now());
        file_store
            .save_migration_approvals(&approvals)
            .with_context(|| "Failed to save migration approval store")?;
        render(&canonical, &plan, &hash, previously_approved, args.json);
        return Err(CliError::PendingMigration(format!(
            "smelt migrate: '{}' has no approved plan matching its current definition — review \
             the plan above and run `smelt migrate {} --apply` again to approve it",
            canonical, canonical
        ))
        .into());
    }

    // Plan mode: derive, print, and record — the record is what a
    // subsequent `--apply` checks freshness against, not itself an
    // approval; exit 3 flags the pending, unreviewed migration to CI
    // (`docs/specs/cli.md` §"Exit codes").
    approvals.record(&canonical, hash.clone(), Utc::now());
    file_store
        .save_migration_approvals(&approvals)
        .with_context(|| "Failed to save migration approval store")?;
    render(&canonical, &plan, &hash, previously_approved, args.json);
    Err(CliError::PendingMigration(format!(
        "smelt migrate: '{}' has a non-trivial migration plan pending — review the plan above \
         and run `smelt migrate {} --apply` to approve and (once execution lands) apply it",
        canonical, canonical
    ))
    .into())
}

fn render(model_name: &str, plan: &MigrationPlan, hash: &str, approved: bool, json: bool) {
    if json {
        print_plan_json(model_name, plan, hash, approved);
    } else {
        print_plan(model_name, plan);
        println!("\nplan hash: {hash}");
    }
}

fn print_plan(model_name: &str, plan: &smelt_logical::backbuild::MigrationPlan) {
    println!("Migration plan for {}", model_name);
    println!("{}", "=".repeat(60));

    if plan.eclipsed {
        println!("eclipsed: nothing to do (definition unchanged)");
        return;
    }

    if plan.groups.is_empty() {
        println!("no column groups derived");
    }

    for group in &plan.groups {
        print_group(group);
    }

    println!();
    println!(
        "full-refresh baseline: {} statement(s)",
        plan.full_refresh.statement_count()
    );
}

fn print_group(group: &ColumnGroupPlan) {
    println!();
    println!("- {}", group.label);
    println!("  verdict: {}", verdict_label(group.verdict));

    if group.candidates.is_empty() {
        println!("  technique: none admissible — full refresh only");
    } else {
        for candidate in &group.candidates {
            print_candidate(candidate);
        }
    }

    for refusal in &group.refusals {
        println!("  refused: {}", refusal.reason);
    }
}

fn print_candidate(candidate: &TechniqueCandidate) {
    println!(
        "  technique: {:?} (cost: {}, {} statement(s), rerun-safe: {})",
        candidate.technique,
        cost_class_label(candidate.cost_class),
        candidate.statement_count(),
        candidate.rerun_safe
    );
}

fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Eclipsed => "eclipsed",
        Verdict::BackfillInPlace => "backfill in place",
        Verdict::ReDerive => "re-derive",
        Verdict::SkeletonChange => "skeleton change (rebuild required)",
    }
}

fn cost_class_label(cost_class: CostClass) -> &'static str {
    match cost_class {
        CostClass::Metadata => "metadata-only",
        CostClass::LocalColumnUpdate => "local column update",
        CostClass::UpstreamColumnUpdate => "upstream column update",
        CostClass::LocalRowSubset => "local row subset",
        CostClass::UpstreamRowSubset => "upstream row subset",
        CostClass::Destructive => "destructive",
        CostClass::FullTable => "full table",
    }
}

/// `--json` rendering (CI mode, `docs/specs/definition_deltas.md` §Surface
/// "`smelt migrate`"). The plan types stay serde-free
/// (`smelt-logical`/`smelt-runtime` purity) — this builds a
/// [`serde_json::Value`] directly in the CLI, the one layer allowed to know
/// about JSON shape.
fn print_plan_json(model_name: &str, plan: &MigrationPlan, hash: &str, approved: bool) {
    let value = serde_json::json!({
        "model": model_name,
        "eclipsed": plan.eclipsed,
        "plan_hash": hash,
        "approved": approved,
        "groups": plan.groups.iter().map(group_json).collect::<Vec<_>>(),
        "full_refresh": {
            "technique": format!("{:?}", plan.full_refresh.technique),
            "statement_count": plan.full_refresh.statement_count(),
        },
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"))
    );
}

fn group_json(group: &ColumnGroupPlan) -> serde_json::Value {
    serde_json::json!({
        "label": group.label,
        "verdict": verdict_code(group.verdict),
        "candidates": group.candidates.iter().map(candidate_json).collect::<Vec<_>>(),
        "refusals": group.refusals.iter().map(|r| serde_json::json!({
            "atom": r.atom,
            "reason": r.reason,
        })).collect::<Vec<_>>(),
    })
}

fn candidate_json(candidate: &TechniqueCandidate) -> serde_json::Value {
    serde_json::json!({
        "technique": format!("{:?}", candidate.technique),
        "cost_class": cost_class_label(candidate.cost_class),
        "statement_count": candidate.statement_count(),
        "reads_upstream": candidate.reads_upstream,
        "rerun_safe": candidate.rerun_safe,
    })
}

fn verdict_code(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Eclipsed => "eclipsed",
        Verdict::BackfillInPlace => "backfill_in_place",
        Verdict::ReDerive => "re_derive",
        Verdict::SkeletonChange => "skeleton_change",
    }
}
