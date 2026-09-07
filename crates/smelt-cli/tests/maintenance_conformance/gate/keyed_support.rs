//! Staging and row-mutation helpers for the `grain: key` pool (`KeyedRecipe`), in both the window-forward and snapshot-reconcile postures.

use smelt_maintenance_testkit::link_c_harness::LinkCProject;
use smelt_maintenance_testkit::recipe::KeyedRecipe;
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::schedule_gen::GenRow;

// ---------------------------------------------------------------------
// Phase 5: the `grain: key` pool (`KeyedRecipe`).
// ---------------------------------------------------------------------

/// Default deterministic case count for `keyed_pool_upholds_end_state_equivalence`
/// — small since each case drives several `execute_project` windows.
pub(crate) const KEYED_DEFAULT_CASES: usize = 6;

pub(crate) fn keyed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_KEYED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(KEYED_DEFAULT_CASES)
}

/// Stage a [`KeyedRecipe`] into a fresh temp project + DuckDB file.
pub(crate) fn stage_keyed_recipe(
    recipe: &KeyedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_keyed(recipe, &project_dir, &db_path)
}

/// [`stage_keyed_recipe`], additionally staging a downstream `SELECT * FROM
/// smelt.<model>` consumer model (`render::stage_keyed_with_downstream`,
/// phase 8 task 5) — opt-in so every pre-existing keyed recipe's staged
/// project shape stays byte-identical.
pub(crate) fn stage_keyed_recipe_with_downstream(
    recipe: &KeyedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_keyed_with_downstream(recipe, &project_dir, &db_path)
}

/// Stage a [`KeyedRecipe`] built over
/// [`SourceRecipe::unclocked_append_only_dimension`] — [`stage_keyed_recipe`]
/// (via `render::stage_keyed`) always emits its `AppendOnly` source's
/// standard `source_yaml()`, which unconditionally declares a `timeseries:`
/// block; this probe needs an `AppendOnly`-postured source with NO
/// `timeseries:` block anywhere in the model (the model/classifier
/// plan-agreement finding, `docs/plans/20260809-keyed-frontier.md` Phase 3
/// review), so it writes a bespoke source YAML directly — same physical
/// `(d DATE, id INTEGER, val INTEGER)` shape `stage_keyed`'s `AppendOnly`
/// DDL branch already expects (mirrors `stage_mixed_recipe`'s own
/// bespoke-YAML staging above for the same reason).
pub(crate) fn stage_keyed_unclocked_append_only(
    recipe: &KeyedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        render::render_keyed_model_file(recipe),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.source.name)),
        format!(
            "description: plan/classifier-agreement probe source, append-only with no \
             declared timeseries block.\n\
             mutation_profile: append_only\n\
             columns:\n\
             \x20 - name: {d}\n    type: DATE\n\
             \x20 - name: {id}\n    type: INTEGER\n\
             \x20 - name: {val}\n    type: INTEGER\n",
            d = recipe.source.clock_column,
            id = recipe.source.key_column,
            val = recipe.source.payload_column,
        ),
    )?;
    std::fs::write(
        project_dir.join("smelt.yml"),
        render::render_smelt_yml(&db_path),
    )?;

    let conn = duckdb::Connection::open(&db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{name} ({d} DATE, {id} INTEGER, {val} INTEGER);",
        name = recipe.source.name,
        d = recipe.source.clock_column,
        id = recipe.source.key_column,
        val = recipe.source.payload_column,
    ))?;
    drop(conn);

    LinkCProject::load(project_dir, db_path)
}

/// Insert one row into a [`KeyedRecipe`]'s staged driving-source table.
pub(crate) fn insert_row_keyed(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.source.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val_sql(),
        ),
        [],
    )?;
    Ok(())
}

/// Insert one row into a snapshot-reconcile [`KeyedRecipe`]'s staged
/// (unclocked, `mutable_snapshot`) driving-source table — `(id, attr)`, no
/// clock column (`SourceRecipe::mutable_dimension`'s shape, unlike
/// [`insert_row_keyed`]'s clocked `(d, id, val)`).
pub(crate) fn insert_row_keyed_snapshot(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    id: i64,
    attr: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES ({id}, {attr})",
            recipe.source.name
        ),
        [],
    )?;
    Ok(())
}

/// Update a snapshot-reconcile [`KeyedRecipe`]'s staged dimension row's
/// `attr` column.
pub(crate) fn update_row_keyed_snapshot(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    id: i64,
    attr: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "UPDATE main.sources_{} SET {} = {attr} WHERE {} = {id}",
            recipe.source.name, recipe.source.payload_column, recipe.source.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Delete a snapshot-reconcile [`KeyedRecipe`]'s staged dimension row — the
/// genuine-departure case: `id` must be RETAINED, unchanged, in the
/// maintained table after the next run (`incremental_shapes.md` §"The two
/// run shapes" — snapshot-reconcile never deletes a departed key).
pub(crate) fn delete_row_keyed_snapshot(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    id: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "DELETE FROM main.sources_{} WHERE {} = {id}",
            recipe.source.name, recipe.source.key_column,
        ),
        [],
    )?;
    Ok(())
}
