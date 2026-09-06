//! The composed (`grain: key` + `timeseries:`) family's classification helpers and route 1 (key-embedded) drive through the real `execute_project` pipeline.

use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{ComposedKeyedRecipe, ComposedRoute, KeyedSchedule};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::schedule_gen::GenRow;

// ---------------------------------------------------------------------
// Phase A6: the composed (`grain: key` + `timeseries:`) recipe family,
// covering all three key-temporal-locality routes
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase A6;
// `incremental_shapes.md` §"Key temporal locality").
//
// Route 1 (key-embedded) is driven through the real `execute_project`
// pipeline, exactly like the keyed pool above. Routes 2 (key-determined)
// and 3 (recurrence-bounded, declared) are admitted by the real
// `establish_locality` gate over real staged frontmatter/YAML
// (`classify_composed_full`), but drive their actual merge mechanics
// through `run_windowed_keyed_maintenance` directly against a real
// `DuckDbBackend` — the documented, pre-existing workaround
// `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs` already
// uses (see `ComposedKeyedRecipe`'s own doc comment for why).
// ---------------------------------------------------------------------

/// Default deterministic case count for `composed_keyed_pool_upholds_equivalence`.
pub(crate) const COMPOSED_DEFAULT_CASES: usize = 6;

pub(crate) fn composed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_COMPOSED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(COMPOSED_DEFAULT_CASES)
}

/// Classify a staged [`ComposedKeyedRecipe`] through the real maintenance
/// derivation — the composed-pool counterpart of `classify_keyed_full`.
/// Returns the derived plan (possibly with zero cells / a locality refusal)
/// plus every diagnostic on the target model.
pub(crate) fn classify_composed_full(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<(
    Option<smelt_logical::maintenance::MaintenancePlan>,
    Vec<smelt_db::Diagnostic>,
)> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project
        .project_dir
        .join(format!("models/{}.sql", recipe.model_name));

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project.project_dir.clone(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file = db.set_source_file(
                m.path.clone(),
                m.content.clone(),
                project.project_dir.clone(),
            );
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project_input]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged composed-pool model {:?} (expected at {}) not found among discovered models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// Assert `recipe`'s plan clears the locality gate with the expected
/// [`LocalitySlice`] shape for its own [`ComposedRoute`] — the single
/// per-case admission check every drive path below relies on having
/// already passed.
pub(crate) fn assert_composed_admitted_with_expected_route(
    recipe: &ComposedKeyedRecipe,
    plan: &smelt_logical::maintenance::MaintenancePlan,
) -> anyhow::Result<()> {
    if plan.refusals.iter().any(|r| {
        matches!(
            r,
            smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
        )
    }) {
        anyhow::bail!(
            "composed recipe {:?} (route {:?}) was refused by the locality gate: {:?}",
            recipe.model_name,
            recipe.route,
            plan.refusals
        );
    }
    let Some(key_locality) = plan.key_locality.as_ref() else {
        anyhow::bail!(
            "composed recipe {:?} (route {:?}) admitted a plan with no key_locality",
            recipe.model_name,
            recipe.route
        );
    };
    match (recipe.route, &key_locality.slice) {
        (ComposedRoute::KeyEmbedded, LocalitySlice::Window { .. }) => Ok(()),
        (ComposedRoute::KeyDetermined, LocalitySlice::DeltaValues { .. }) => Ok(()),
        (ComposedRoute::KeyDerived, LocalitySlice::DeltaValues { .. }) => Ok(()),
        (ComposedRoute::RecurrenceBounded, LocalitySlice::RecurrenceBounded { .. }) => Ok(()),
        (route, slice) => {
            anyhow::bail!(
                "composed recipe {:?}: route {:?} admitted an unexpected slice shape: {:?}",
                recipe.model_name,
                route,
                slice
            )
        }
    }
}

// ---- Route 1 (key-embedded): full `execute_project` drive -----------

pub(crate) fn insert_composed_row(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
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

/// Whole-table equivalence for route 1: the maintained output equals the
/// model body evaluated over the full, currently-inserted source table —
/// route 1's schedule never reprocesses a window, so no `STracker`
/// S-restriction is needed.
pub(crate) async fn assert_composed_route1_equivalence(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = render::render_composed_oracle_sql(recipe);
    if !multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await? {
        anyhow::bail!(
            "composed route-1 equivalence violated for {:?}: maintained ({maintained_sql:?}) != \
             oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Per-slice equivalence for route 1 (`incremental_shapes.md` §"Key temporal
/// locality (the time-partitioned output)"): the stored rows of one output slice (`d = slice_date`)
/// equal the model SQL evaluated over the source rows within that slice's
/// derived reach — zero margin here (`SIMPLE_SQL`-shaped, no lookback), so
/// the reach is exactly the source rows sharing that same date.
pub(crate) async fn assert_composed_route1_per_slice(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
    slice_date: chrono::NaiveDate,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    let d = slice_date.format("%Y-%m-%d");
    let maintained_sql = format!(
        "SELECT * FROM main.{} WHERE d = DATE '{d}'",
        recipe.model_name
    );
    let oracle_body = render::render_composed_oracle_sql(recipe);
    let oracle_sql = format!("SELECT * FROM ({oracle_body}) t WHERE d = DATE '{d}'");
    if !multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await? {
        anyhow::bail!(
            "composed route-1 per-slice equivalence violated for {:?} at slice {d}: maintained \
             ({maintained_sql:?}) != model SQL over the slice's derived reach ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

pub(crate) async fn drive_composed_route1_and_assert(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
    schedule: &KeyedSchedule,
) -> anyhow::Result<()> {
    for (i, window) in schedule.0.iter().enumerate() {
        for row in &window.rows {
            insert_composed_row(project, recipe, row)?;
        }

        let mut request = base_request("dev");
        request.start = Some(window.start.format("%Y-%m-%d").to_string());
        request.end = Some(window.end.format("%Y-%m-%d").to_string());
        project
            .run_quiet(&format!("composed-route1-run-{i}"), request)
            .await?;

        assert_composed_route1_equivalence(project, recipe).await?;
        assert_composed_route1_per_slice(project, recipe, window.start).await?;
    }
    Ok(())
}
