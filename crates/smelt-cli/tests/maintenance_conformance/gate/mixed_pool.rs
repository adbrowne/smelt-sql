//! The fact+mutable-dimension mixed pool (`MutableEnrichedRecipe`): staging, classification, the settled-point oracle, and the pool's proptest gate.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::oracle_modes::OracleMode;
use smelt_maintenance_testkit::recipe::MutableEnrichedRecipe;
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{
    arb_mixed_schedule, check_profile as check_mixed_schedule_profile, read_source_snapshot,
    GenRow, MixedSchedule, MixedStep,
};

// ---------------------------------------------------------------------
// Phase 4: the fact+mutable-dimension mixed pool (`MutableEnrichedRecipe`).
// ---------------------------------------------------------------------

/// Default deterministic case count for `mutable_pool_settles_to_full_refresh`
/// — smaller than [`DEFAULT_CASES`] since each case also drives a dimension
/// mutation + catch-up run on top of the fact-window runs.
pub(crate) const MIXED_DEFAULT_CASES: usize = 6;

pub(crate) fn mixed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_MIXED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MIXED_DEFAULT_CASES)
}

/// Generous upper bound on distinct fact/dimension ids a generated
/// [`MixedSchedule`] can ever produce (`arb_mixed_schedule`'s shape: at most
/// 3 windows × 2 rows), so [`stage_mixed_recipe`] can pre-seed every
/// dimension row a schedule might reference (an unmatched fact id would
/// silently vanish from the INNER JOIN output, which is a staging bug, not a
/// case the pool ever intends to reach).
pub(crate) const MIXED_DIM_SEED_MAX_ID: i64 = 12;

/// Stage a [`MutableEnrichedRecipe`] into a fresh temp project + DuckDB file:
/// writes both source YAMLs + the model file, creates both physical source
/// tables, and pre-seeds the dimension table with one row per id in
/// `1..=MIXED_DIM_SEED_MAX_ID` (`attr = id * 100`) so every fact row a
/// generated schedule inserts already has a matching dimension row to join
/// against.
pub(crate) fn stage_mixed_recipe(
    recipe: &MutableEnrichedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        recipe.model_file(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.fact.name)),
        recipe.fact_source_yaml(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.dimension.name)),
        recipe.dimension_source_yaml(),
    )?;
    std::fs::write(
        project_dir.join("smelt.yml"),
        render::render_smelt_yml(&db_path),
    )?;

    let conn = duckdb::Connection::open(&db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{fact} ({d} DATE, {id} INTEGER, {val} INTEGER); \
         CREATE TABLE main.sources_{dim} ({dim_id} INTEGER, {attr} INTEGER);",
        fact = recipe.fact.name,
        d = recipe.fact.clock_column,
        id = recipe.fact.key_column,
        val = recipe.fact.payload_column,
        dim = recipe.dimension.name,
        dim_id = recipe.dimension.key_column,
        attr = recipe.dimension.payload_column,
    ))?;
    for id in 1..=MIXED_DIM_SEED_MAX_ID {
        conn.execute(
            &format!(
                "INSERT INTO main.sources_{} VALUES ({}, {})",
                recipe.dimension.name,
                id,
                id * 100
            ),
            [],
        )?;
    }
    drop(conn);

    LinkCProject::load(project_dir, db_path)
}

/// Insert one fact row into `recipe`'s staged fact source table.
pub(crate) fn insert_fact_row(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.fact.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val_sql(),
        ),
        [],
    )?;
    Ok(())
}

/// Classify a staged [`MutableEnrichedRecipe`] through the real maintenance
/// derivation — the mixed-pool counterpart of `verdict::classify`, kept here
/// rather than in `verdict.rs` (outside this phase's edit scope, plan
/// Critical files) since `verdict::classify` only accepts a `ModelRecipe`.
pub(crate) fn classify_mixed(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
) -> anyhow::Result<smelt_logical::maintenance::MaintenancePlan> {
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
            "staged mixed-pool model {:?} (expected at {}) not found among discovered models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let plan_result =
        smelt_db::maintenance_plan_report(&db, workspace, target).ok_or_else(|| {
            anyhow::anyhow!(
                "no maintenance plan report for mixed-pool model {:?}",
                recipe.model_name
            )
        })?;
    Ok(plan_result.plan)
}

/// The window a `MutateDimension` step's `id` was produced in, by scanning
/// the schedule's own `RunWindow` steps — the same lookup rule
/// `schedule_gen::check_profile` uses.
pub(crate) fn fact_window_for_id(
    schedule: &MixedSchedule,
    id: i64,
) -> Option<(chrono::NaiveDate, chrono::NaiveDate)> {
    schedule.0.iter().find_map(|s| match s {
        MixedStep::RunWindow { start, end, rows } => {
            rows.iter().any(|r| r.id == id).then_some((*start, *end))
        }
        _ => None,
    })
}

/// The settled-point oracle assertion (design §6 "Mixed models"): the fact
/// side reads the S-restricted temp table (`tracker`'s `S_k`); the
/// dimension side always reads its CURRENT physical state.
pub(crate) async fn assert_mixed_settled(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    tracker.materialize_s(backend.as_ref(), k).await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    // `STracker::materialize_s` names its temp table `oracle_<source_name>`
    // (an internal convention of `s_tracker.rs`, mirrored here rather than
    // exposed as a public accessor since this is the only Phase 4 call site
    // needing it).
    let oracle_sql = recipe.oracle_body_over(&format!("oracle_{}", recipe.fact.name));
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "settled-point equivalence violated for {:?} at run {k}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` (a [`MutableEnrichedRecipe`])
/// through the real `execute_project` pipeline: every `RunWindow` step
/// inserts its rows then runs; every `MutateDimension` step mutates the
/// dimension in place and records the affected window as outstanding
/// (`STracker::record_dimension_mutation`). Equivalence is asserted after
/// every `RunWindow` step ONLY while the tracker reports
/// [`OracleMode::SRestricted`] — the weaker expected-staleness contract
/// while [`OracleMode::SettledPoint`] holds is recorded non-fatally, never
/// asserted as a hard failure (design §6 "Mixed models").
pub(crate) async fn drive_mixed_and_assert(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
    schedule: &MixedSchedule,
) -> anyhow::Result<()> {
    let mut tracker = STracker::new(&recipe.fact);
    let mut settled_assertions = 0usize;

    for (i, step) in schedule.0.iter().enumerate() {
        match step {
            MixedStep::RunWindow { start, end, rows } => {
                for row in rows {
                    insert_fact_row(project, recipe, row)?;
                }

                let mut request = base_request("dev");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project
                    .run_quiet(&format!("mixed-run-{i}"), request)
                    .await?;

                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.fact)
                };
                let k = tracker.record_run(*start, *end, snapshot);

                match tracker.oracle_mode() {
                    OracleMode::SRestricted => {
                        assert_mixed_settled(project, recipe, &tracker, k).await?;
                        settled_assertions += 1;
                    }
                    OracleMode::SettledPoint => {
                        // Expected staleness: some window's dimension
                        // mutation has not yet been caught up — non-fatal
                        // (design §6).
                    }
                }
            }
            MixedStep::MutateDimension { id, new_attr } => {
                let conn = project.connect()?;
                conn.execute(
                    &format!(
                        "UPDATE main.sources_{} SET {} = {} WHERE {} = {}",
                        recipe.dimension.name,
                        recipe.dimension.payload_column,
                        new_attr,
                        recipe.dimension.key_column,
                        id,
                    ),
                    [],
                )?;
                drop(conn);

                let window = fact_window_for_id(schedule, *id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "MutateDimension targets id {id} which no RunWindow step produced \
                         (schedule_gen::check_profile should have caught this)"
                    )
                })?;
                tracker.record_dimension_mutation(window);
            }
        }
    }

    anyhow::ensure!(
        settled_assertions > 0,
        "schedule {schedule:?} never reached a settled point — no assertion ran"
    );
    Ok(())
}

/// `mutable_pool_settles_to_full_refresh` (plan Phase 4 TDD list):
/// fact+mutable-dimension recipes (the `daily_events_enriched` shape,
/// generated schedules) hold full equivalence at every settled point;
/// expected-staleness in between is recorded, never fatal.
#[test]
fn mutable_pool_settles_to_full_refresh() {
    let n = mixed_case_count();
    let mut runner = TestRunner::deterministic();
    let recipe = MutableEnrichedRecipe::new();
    let schedule_strat = arb_mixed_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..n {
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        check_mixed_schedule_profile(&schedule).unwrap_or_else(|e| {
            panic!("case {i}: generated mixed schedule {schedule:?} failed its own self-check: {e}")
        });

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_mixed_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: failed to stage mixed recipe: {e}"));

        let plan = classify_mixed(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: classify_mixed failed: {e}"));
        assert!(
            !plan.cells.is_empty(),
            "case {i}: mixed recipe {recipe:?} admitted zero cells — generator/derivation \
             regression (expected at least the UpstreamMutation ColumnScopedMerge cell)"
        );

        rt.block_on(drive_mixed_and_assert(&project, &recipe, &schedule))
            .unwrap_or_else(|e| {
                panic!("case {i}: mixed schedule {schedule:?} equivalence check failed: {e}")
            });
    }
}
