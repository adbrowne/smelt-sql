//! `smelt migrate <model>` — plan-and-approve definition-delta migration verb
//! (`docs/specs/definition_deltas.md` §"`smelt migrate`").
//!
//! The plan step derives a [`smelt_logical::backbuild::DefinitionDiff`]
//! between the model's last-deployed SQL (`DeployedSchema::model_sql`) and
//! its current SQL, classifies it via `smelt_logical::backbuild`, prints a
//! per-column-group migration plan (verdict + technique + plan hash), and
//! records the plan hash to a per-target approval store — this is what
//! "seeing the plan printed" means for approval purposes. `--apply` executes
//! only a plan whose freshly re-derived hash matches the recorded one.

use anyhow::{Context, Result};
use thiserror::Error;

use smelt_cli::{find_project_root, Config, ModelDiscovery, SourcesConfig};
use smelt_logical::backbuild::{ColumnGroupPlan, MigrationPlan, MigrationVerdict};
use smelt_runtime::definition_delta::{apply_migration, derive_plan};
use smelt_state::file_store::FileStore;
use smelt_state::migration_approvals::MigrationApprovalStore;

use crate::helpers::infer_deployed_columns;
use crate::MigrateArgs;

/// `smelt migrate`'s own exit-code classification
/// (`docs/specs/cli.md` §"Exit codes" — "`smelt migrate` specifics"). See
/// [`exit_code_for`].
#[derive(Debug, Error)]
enum MigrateError {
    /// The plan step derived a non-eclipsed plan that has not previously
    /// been recorded as approved. Exit `3`.
    #[error("{0}")]
    PendingApproval(String),
    /// `--apply` found no matching approval on record. Exit `3`.
    #[error("{0}")]
    ApplyRefused(String),
    /// The plan (or its interrupted resume) requires a full refresh. Exit
    /// `1`.
    #[error("{0}")]
    FullRefreshRequired(String),
}

/// Classify a top-level command error for `smelt migrate`'s exit code: `3`
/// for a pending-approval or apply-refusal state, `1` for a
/// full-refresh-required state, else the shared classifier. Same pattern as
/// `commands::list::exit_code_for`.
pub fn exit_code_for(err: &anyhow::Error) -> u8 {
    match err.downcast_ref::<MigrateError>() {
        Some(MigrateError::PendingApproval(_)) | Some(MigrateError::ApplyRefused(_)) => 3,
        Some(MigrateError::FullRefreshRequired(_)) => 1,
        None => smelt_cli::exit_code_for(err),
    }
}

pub async fn migrate(args: MigrateArgs, _scope: Option<&str>) -> Result<()> {
    // 1. Resolve project root + config + target.
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;
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

    // 5-7. Parse both sides, derive the definition diff, build
    // `BackbuildInputs`, and derive the plan — the single shared derivation
    // `smelt run`'s pending-migration gate and `smelt explain` also read
    // (`smelt_runtime::definition_delta::derive_plan`). No execution
    // happens below unless `--apply` is set.
    let db = smelt_cli::init_db(&project_dir, &models);
    let derived = derive_plan(
        &file_store,
        model,
        &models,
        sources.as_ref(),
        &db,
        &before_sql_raw,
        &deployed.columns,
    )?;
    let after_clean = derived.inputs.after_sql.clone();
    let inferred = infer_deployed_columns(&db, model);
    let plan = derived.plan;
    let hash = derived.hash;

    let mut approvals = file_store
        .load_migration_approvals()
        .with_context(|| "Failed to load migration-approval store")?;

    if args.apply {
        apply_plan(
            &project_dir,
            &file_store,
            target_config,
            &db,
            model,
            &db_name,
            &model_name,
            &after_clean,
            deployed.version,
            deployed.partition_column.as_deref(),
            &inferred,
            &plan,
            &hash,
            &mut approvals,
            args.json,
        )
        .await
    } else {
        let approved_before = plan_step(&model_name, &plan, &hash, &mut approvals, args.json);
        file_store
            .save_migration_approvals(&approvals)
            .with_context(|| "Failed to save migration-approval store")?;

        if plan.groups.is_empty() || approved_before {
            return Ok(());
        }
        Err(MigrateError::PendingApproval(format!(
            "smelt migrate: '{model_name}' has a new, unapproved migration plan (printed \
             above). Review it, then run `smelt migrate {model_name} --apply` to execute it."
        ))
        .into())
    }
}

/// Whether `model`'s recorded approval, if any, already matches `hash` —
/// computed before the caller's own record-and-save, so it reflects a
/// *previous* invocation having seen this exact plan.
fn already_approved(approvals: &MigrationApprovalStore, model: &str, hash: &str) -> bool {
    approvals.get(model).is_some_and(|a| a.plan_hash == hash)
}

/// The plan-only path: render the plan (human or JSON), record the hash
/// unconditionally, and return whether this exact plan was already on
/// record *before* this call (used for the exit-code decision).
fn plan_step(
    model_name: &str,
    plan: &MigrationPlan,
    hash: &str,
    approvals: &mut MigrationApprovalStore,
    json: bool,
) -> bool {
    let approved_before = already_approved(approvals, model_name, hash);

    if json {
        render_json(model_name, plan, hash, approved_before);
    } else {
        render_plan(model_name, plan, hash);
    }

    approvals.record(model_name, hash.to_string(), false);
    approved_before
}

#[allow(clippy::too_many_arguments)]
async fn apply_plan(
    project_dir: &std::path::Path,
    file_store: &FileStore,
    target_config: &smelt_cli::config::Target,
    db: &smelt_db::Database,
    model: &smelt_cli::ModelFile,
    db_name: &str,
    model_name: &str,
    after_sql: &str,
    deployed_version: u32,
    partition_column: Option<&str>,
    inferred: &[smelt_state::schema_tracking::DeployedColumn],
    plan: &MigrationPlan,
    hash: &str,
    approvals: &mut MigrationApprovalStore,
    json: bool,
) -> Result<()> {
    let _ = db;
    let _ = model;

    let recorded = approvals.get(model_name).cloned();

    let Some(recorded) = recorded else {
        render_plan(model_name, plan, hash);
        return Err(MigrateError::ApplyRefused(format!(
            "smelt migrate --apply: no approved plan is on record for '{model_name}' — run \
             `smelt migrate {model_name}` first to derive and approve one."
        ))
        .into());
    };

    if recorded.plan_hash != *hash {
        render_plan(model_name, plan, hash);
        return Err(MigrateError::ApplyRefused(format!(
            "smelt migrate --apply: the approved plan for '{model_name}' is stale — the \
             definition or its inputs changed since it was approved. The freshly derived plan \
             is printed above; review it and run `smelt migrate {model_name}` again to approve \
             it before applying."
        ))
        .into());
    }

    if recorded.in_progress && !plan.all_rerun_safe() {
        return Err(MigrateError::FullRefreshRequired(format!(
            "smelt migrate --apply: an earlier apply of '{model_name}' was interrupted \
             mid-execution, and this plan's chosen technique is not safely re-runnable from the \
             start — resuming could re-apply a non-idempotent statement. Use a full refresh \
             instead: `smelt run --allow-full-refresh {model_name}`."
        ))
        .into());
    }

    if plan.statements.is_empty() {
        return Err(MigrateError::FullRefreshRequired(format!(
            "smelt migrate --apply: '{model_name}' has no admissible in-place technique for \
             this migration — a full refresh is the only route: \
             `smelt run --allow-full-refresh {model_name}`."
        ))
        .into());
    }

    approvals.record(model_name, hash.to_string(), true);
    file_store
        .save_migration_approvals(approvals)
        .with_context(|| "Failed to save migration-approval store")?;

    let backend = crate::helpers::create_backend(target_config, project_dir, None)
        .await
        .with_context(|| "Failed to connect to target backend")?;

    apply_migration(backend.as_ref(), plan).await?;

    smelt_runtime::schema_evolution::save_deployed_schema(
        file_store,
        db_name,
        after_sql,
        inferred,
        Some(deployed_version),
        partition_column,
    )
    .with_context(|| format!("Failed to record the migrated schema for {model_name}"))?;

    approvals.clear(model_name);
    file_store
        .save_migration_approvals(approvals)
        .with_context(|| "Failed to save migration-approval store")?;

    if json {
        render_json(model_name, plan, hash, true);
    } else {
        println!(
            "smelt migrate {model_name}: applied {} statement{} — the definition delta is \
             cleared.",
            plan.statements.len(),
            if plan.statements.len() == 1 { "" } else { "s" }
        );
    }

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

fn render_json(model_name: &str, plan: &MigrationPlan, hash: &str, approved: bool) {
    use serde_json::json;

    let verdict_label = |v: MigrationVerdict| match v {
        MigrationVerdict::Eclipsed => "eclipsed",
        MigrationVerdict::BackfillInPlace => "backfill_in_place",
        MigrationVerdict::Rederive => "rederive",
        MigrationVerdict::SkeletonChange => "skeleton_change",
    };

    let groups: Vec<_> = plan
        .groups
        .iter()
        .map(|group| {
            let (technique, statement_count) = match group.options.first() {
                Some(option) => (
                    Some(format!("{:?}", option.technique)),
                    option.statement_count(),
                ),
                None => (None, 0),
            };
            json!({
                "columns": group.columns,
                "verdict": verdict_label(group.verdict),
                "technique": technique,
                "statement_count": statement_count,
                "refusals": group.refusals.iter().map(|r| r.reason.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();

    let output = json!({
        "model": format!("smelt.{model_name}"),
        "table": plan.table,
        "verdict": verdict_label(plan.verdict()),
        "plan_hash": hash,
        "approved": approved,
        "groups": groups,
        "statements": plan.statements,
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("JSON serialization should not fail")
    );
}
