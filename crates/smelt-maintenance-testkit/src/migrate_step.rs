//! The shared `ConformanceStep::MigrateModel` driver
//! (`docs/outcomes/20260815-definition-delta-migrate/phases/05-plan.md`
//! task 3): rewrite the model file on disk, derive the migration plan
//! against the model's last-deployed definition through the exact same
//! [`smelt_runtime::definition_delta::derive_plan`] `smelt migrate` itself
//! calls, then either execute the derived in-place statements
//! ([`smelt_runtime::definition_delta::apply_migration`]) or fall back to an
//! ordinary full-refresh run when the plan admits no in-place technique.
//!
//! Deliberately routed through the *backbuild* derivation
//! (`smelt_logical::backbuild`) rather than the live maintenance driver's
//! own `Trigger::ColumnAdded` dispatch (`smelt_runtime::maintenance_driver`)
//! — those are two distinct mechanisms (`docs/specs/definition_deltas.md`'s
//! "narrower third mechanism" note); this module exercises the one `smelt
//! migrate` actually ships, since that's what
//! `schedule_gen::ConformanceStep::MigrateModel` claims to stage.
//!
//! Shared by both `crates/smelt-cli/tests/maintenance_conformance/gate.rs`
//! and `crate::families::gate`'s target-parametrized twin — the
//! derive→classify→apply-or-full-refresh core never re-derives between the
//! two call sites; only the actual full-refresh *run* invocation (which
//! differs between `LinkCProject::run_quiet` and `LinkCProject::run_with_target`)
//! is left to the caller via the `full_refresh` closure.

use std::future::Future;
use std::path::Path;

use anyhow::{Context, Result};

use smelt_backend::Backend;
use smelt_runtime::definition_delta::{apply_migration, derive_plan};
use smelt_runtime::schema_evolution::{infer_deployed_columns, save_deployed_schema};
use smelt_state::file_store::FileStore;

use crate::recipe::{ModelEdit, ModelRecipe};
use crate::render;

/// Which leg [`run_migrate_step`] actually took — surfaced so gates can
/// assert on it (e.g. "at least one case in the deterministic sample took
/// the `Applied` leg", mirroring `admission_rate_stays_above_floor`'s
/// anti-vacuity discipline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrateStepOutcome {
    /// The derived plan admitted at least one in-place statement; those
    /// statements were executed via [`apply_migration`] and the deployed
    /// schema was updated to reflect the new definition. No new source data
    /// was read — the S-tracker's existing coverage is unaffected.
    Applied,
    /// The derived plan admitted no in-place technique
    /// (`plan.statements.is_empty()`, the same condition
    /// `commands::migrate::apply_plan` reports as
    /// `MigrateError::FullRefreshRequired`) — the caller's `full_refresh`
    /// closure ran instead, recomputing the whole table from the CURRENT
    /// full source contents under the rewritten body.
    FullRefreshed,
}

/// Rewrite `models/<recipe.model_name>.sql` on disk with `edit` applied,
/// derive the migration plan against the model's last-deployed definition
/// (the same [`derive_plan`] call `smelt migrate` itself makes), and either
/// apply it in place or run `full_refresh` — the [`ConformanceStep::MigrateModel`]
/// step body, factored out so `maintenance_conformance/gate.rs` and
/// `families::gate`'s target-parametrized twin share one derivation rather
/// than each re-deriving admission.
///
/// `target` is the `smelt.yml` target name (`"dev"` for every DuckDB-staged
/// recipe in this crate) — [`FileStore::new`]'s own target key, distinct
/// from `smelt_maintenance_testkit::recipe::ConformanceTarget`.
///
/// `full_refresh` is only invoked when the derived plan admits no in-place
/// technique; it must drive an ordinary unwindowed `execute_project` run
/// over the project's CURRENT on-disk state (the two call sites' own
/// `FullRefreshRun` step arms already do exactly this) — this function
/// itself performs no execution beyond [`apply_migration`]'s in-place
/// statements.
///
/// [`ConformanceStep::MigrateModel`]: crate::schedule_gen::ConformanceStep::MigrateModel
pub async fn run_migrate_step<F, Fut>(
    project_dir: &Path,
    target: &str,
    backend: &dyn Backend,
    recipe: &ModelRecipe,
    edit: ModelEdit,
    full_refresh: F,
) -> Result<MigrateStepOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    // 1. Rewrite the model file on disk.
    let model_path = project_dir.join(format!("models/{}.sql", recipe.model_name));
    std::fs::write(
        &model_path,
        render::render_model_file_with_edit(recipe, edit),
    )
    .with_context(|| format!("write rewritten model file {model_path:?}"))?;

    // 2. Rediscover models and assemble a throwaway `smelt_db::Database`
    // populated the same way `link_c_harness::LinkCProject::build_db_and_graph`
    // and `render::staged_diagnostics` already do — only `derive_plan`'s own
    // `infer_deployed_columns` call needs it.
    let config = smelt_core::config::Config::load(project_dir)
        .with_context(|| format!("load config for {project_dir:?}"))?;
    let discovery =
        smelt_core::ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let all_models = discovery
        .discover_models()
        .with_context(|| "discover models for migrate step")?;
    let model = all_models
        .iter()
        .find(|m| m.name == recipe.model_name)
        .ok_or_else(|| anyhow::anyhow!("model {:?} not discovered", recipe.model_name))?;

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = all_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project_input]);

    // 3. Load the deployed schema + best-effort legacy `sources.yml` (the
    // append-only pool never stages one, mirroring `smelt migrate`'s own
    // `.ok()` fallback to `None`).
    let file_store = FileStore::new(project_dir, target);
    let db_name = model.db_name_owned();
    let deployed = file_store
        .load_schema(&db_name)
        .with_context(|| format!("load deployed schema for {db_name}"))?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no recorded deployed schema for {db_name} — run_migrate_step requires a prior \
                 successful run to have established one"
            )
        })?;
    let before_sql_raw = deployed.model_sql.clone().ok_or_else(|| {
        anyhow::anyhow!("deployed schema for {db_name} predates model_sql tracking")
    })?;
    let sources = smelt_core::sources::SourcesConfig::load(project_dir).ok();

    // 4. Derive the plan — the single shared derivation `smelt migrate`
    // itself calls.
    let derived = derive_plan(
        &file_store,
        model,
        &all_models,
        sources.as_ref(),
        &db,
        &before_sql_raw,
        &deployed.columns,
    )
    .with_context(|| format!("derive migration plan for {db_name}"))?;

    if derived.plan.statements.is_empty() {
        // No admissible in-place technique — the same condition
        // `commands::migrate::apply_plan` reports as
        // `MigrateError::FullRefreshRequired`.
        full_refresh().await?;
        return Ok(MigrateStepOutcome::FullRefreshed);
    }

    apply_migration(backend, &derived.plan)
        .await
        .with_context(|| format!("apply migration plan for {db_name}"))?;

    let inferred = infer_deployed_columns(&db, model);
    save_deployed_schema(
        &file_store,
        &db_name,
        &derived.inputs.after_sql,
        &inferred,
        Some(deployed.version),
    )
    .with_context(|| format!("save migrated deployed schema for {db_name}"))?;

    Ok(MigrateStepOutcome::Applied)
}
