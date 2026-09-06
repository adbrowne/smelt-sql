//! Definition-change steps (`ConformanceStep::RewriteModel`): column adds between runs, the pure-backfill in-place update, and the skeleton-position-add refusal.

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::recipe::ModelRecipe;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{read_source_snapshot, GenRow};

use super::partition_pool::{assert_equivalence, insert_row};

mod column_add_recovery;
mod pure_backfill;
mod skeleton_position_add;

/// Real deployed-schema column names for a staged recipe's model, read
/// straight from the on-disk `FileStore` `smelt-runtime`'s maintenance
/// driver itself reads (`crate::maintenance_driver::
/// resolve_live_in_place_update_cell`'s own doc comment) — never a
/// synthetic stand-in. `None` when no schema has been deployed yet (before
/// the model's first successful run).
pub(crate) fn deployed_column_names(project: &LinkCProject, table: &str) -> Vec<String> {
    let file_store = smelt_state::file_store::FileStore::new(&project.project_dir, "dev");
    file_store
        .load_schema(table)
        .ok()
        .flatten()
        .map(|s| s.columns.into_iter().map(|c| c.name).collect())
        .unwrap_or_default()
}

/// Derive the real production `MaintenancePlan` for `model_name` directly
/// (mirroring `crate::maintenance_driver::resolve_live_in_place_update_cell`'s
/// own input assembly — not a re-derivation of admission, just the same
/// input-gathering a `smelt-db` diagnostics call site cannot do because it
/// has no I/O access to `deployed_column_names`), threading the REAL
/// deployed-schema snapshot read via [`deployed_column_names`].
pub(crate) fn derive_plan_with_real_deployed_schema(
    project: &LinkCProject,
    recipe: &ModelRecipe,
) -> anyhow::Result<smelt_logical::maintenance::MaintenancePlan> {
    use smelt_logical::maintenance::{MutationProfile, SourceFacts};

    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let model = sql_models
        .iter()
        .find(|m| m.name == recipe.model_name)
        .ok_or_else(|| anyhow::anyhow!("model {:?} not discovered", recipe.model_name))?;
    let metadata = model
        .metadata
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("staged recipe model must declare frontmatter"))?;
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let sources = vec![SourceFacts {
        name: recipe.source.name.clone(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some(recipe.source.clock_column.clone()),
        unique_key: vec![],
        allow_full_scan: false,
    }];
    let table = model.db_name_owned();
    let deployed = deployed_column_names(project, &table);
    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        &stripped_sql,
        &table,
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &deployed,
        &std::collections::BTreeMap::new(),
        None,
        None,
        &[],
    )
    .ok_or_else(|| anyhow::anyhow!("model {:?} carries no maintenance plan", recipe.model_name))?;
    Ok(result.plan)
}

/// Shared `RunWindow`-step logic (insert rows, snapshot, run, record,
/// assert equivalence) — factored out of
/// [`pure_backfill::pure_backfill_column_add_executes_in_place_update`]'s
/// creation-run leg so that leg reads identically to `drive_and_assert`'s
/// own `RunWindow` arm.
pub(crate) async fn rt_insert_and_run(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
    rows: &[GenRow],
    tracker: &mut STracker,
) -> anyhow::Result<()> {
    for row in rows {
        insert_row(project, recipe, row).await?;
    }
    let snapshot = {
        let conn = project.connect()?;
        read_source_snapshot(&conn, &recipe.source)
    };
    let mut request = base_request("dev");
    request.start = Some(start.format("%Y-%m-%d").to_string());
    request.end = Some(end.format("%Y-%m-%d").to_string());
    project.run_quiet("creation-run", request).await?;
    let k = tracker.record_run(start, end, snapshot);
    assert_equivalence(project, recipe, tracker, k).await
}
