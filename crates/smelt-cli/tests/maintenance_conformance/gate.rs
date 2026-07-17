//! The standing proptest gate over the append-only partition-grain
//! `ModelRecipe` pool
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 3),
//! plus the fact+mutable-dimension mixed pool (Phase 4).

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::maintenance::{Technique, Trigger};
use smelt_maintenance_testkit::feed::{self, FeedSourcePosture};
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal;
use smelt_maintenance_testkit::oracle_modes::{
    keyed_end_state_with_retained_departed_keys, KeyedOracleRow, OracleMode,
};
use smelt_maintenance_testkit::recipe::{
    arb_composed_route, arb_composed_route3_schedule, arb_keyed_combiner, arb_keyed_schedule,
    arb_recipe, ComposedKeyedRecipe, ComposedRoute, ComposedRoute3Schedule, ConstructKind,
    KeyedCombiner, KeyedRecipe, KeyedSchedule, ModelEdit, ModelRecipe, MutableEnrichedRecipe,
    RecipePool,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{
    arb_mixed_schedule, arb_schedule_for, boundary_rows_for,
    check_profile as check_mixed_schedule_profile, read_source_snapshot, scan_clamp_for,
    ConformanceSchedule, ConformanceStep, GenRow, MixedSchedule, MixedStep,
};
use smelt_maintenance_testkit::verdict::{classify, Verdict};
use smelt_planner::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::check_runner::batches_to_rows;
use smelt_runtime::maintenance_driver::{driving_steps, run_windowed_keyed_maintenance};

/// Default deterministic case count for
/// `append_only_partition_pool_upholds_equivalence` — small enough to stay
/// on par with `property_discovery`'s per-target budget (plan Phase 3
/// Implementation shape: "each case ≈ 1-3s … 12 cases keeps the target on
/// par").
const DEFAULT_CASES: usize = 12;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// Stage `recipe` into a fresh temp project + DuckDB file, returning the
/// loaded [`LinkCProject`].
pub fn stage_recipe(recipe: &ModelRecipe, tmp: &tempfile::TempDir) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage(recipe, &project_dir, &db_path)
}

fn insert_row(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    row: &smelt_maintenance_testkit::schedule_gen::GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.source.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` through the real
/// `execute_project` pipeline, asserting S-restricted multiset equivalence
/// after every `RunWindow` step (design §6). Returns the fully-populated
/// [`STracker`] plus the index of the last recorded run, so
/// `harness_self_check` can reuse the green end-state without re-driving
/// the whole schedule.
pub async fn drive_and_assert(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    schedule: &ConformanceSchedule,
) -> anyhow::Result<(STracker, usize)> {
    let mut tracker = STracker::new(&recipe.source);
    let mut last_k: Option<usize> = None;
    // The most recent `RewriteModel` step's edit (Phase 9), or `None` before
    // any rewrite — threads into every subsequent assertion so the oracle
    // re-renders against the CURRENT on-disk body, never the pre-rewrite one
    // (plan Phase 9 review checklist).
    let mut current_edit: Option<ModelEdit> = None;

    for (i, step) in schedule.0.iter().enumerate() {
        match step {
            ConformanceStep::RunWindow { start, end, rows } => {
                for row in rows {
                    insert_row(project, recipe, row)?;
                }

                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.source)
                };

                let mut request = base_request("dev");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project.run_quiet(&format!("run-{i}"), request).await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);

                assert_equivalence_with_edit(project, recipe, &tracker, k, current_edit)?;
            }
            ConformanceStep::AppendLateRow(row) => {
                insert_row(project, recipe, row)?;
            }
            ConformanceStep::RerunWindow { start, end } => {
                // Redelivery: same window as an earlier `RunWindow`, no new
                // rows. Never-fold-twice under the partition-grain
                // DELETE+INSERT full-replace technique must hold (Phase 6).
                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.source)
                };

                let mut request = base_request("dev");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project.run_quiet(&format!("run-{i}"), request).await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);

                assert_equivalence_with_edit(project, recipe, &tracker, k, current_edit)?;
            }
            ConformanceStep::FullRefreshRun => {
                // Unwindowed run: `execute_project` takes the full-refresh
                // arm (drop + rebuild from the CURRENT full source
                // contents) whenever no `start`/`end` is supplied (Phase 6).
                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.source)
                };

                let mut request = base_request("dev");
                request.full_refresh = true;
                request.start = None;
                request.end = None;
                project.run_quiet(&format!("run-{i}"), request).await?;

                let k = tracker.record_full_refresh(snapshot);
                last_k = Some(k);

                assert_equivalence_with_edit(project, recipe, &tracker, k, current_edit)?;
            }
            ConformanceStep::BackfillRegion { start, end } => {
                // An explicit backfill: same execution shape as `RunWindow`
                // with no accompanying insert (Phase 6).
                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.source)
                };

                let mut request = base_request("dev");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project.run_quiet(&format!("run-{i}"), request).await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);

                assert_equivalence_with_edit(project, recipe, &tracker, k, current_edit)?;
            }
            ConformanceStep::RewriteModel { edit } => {
                // Definition change (Phase 9): rewrite the model file on
                // disk with `edit` applied. No run happens here — the next
                // *Window step re-discovers the model from disk (already
                // the harness's behaviour) and compiles/executes whatever
                // SQL is now on disk.
                let model_path = project
                    .project_dir
                    .join(format!("models/{}.sql", recipe.model_name));
                std::fs::write(
                    &model_path,
                    render::render_model_file_with_edit(recipe, *edit),
                )?;
                current_edit = Some(*edit);
            }
        }
    }

    let last_k =
        last_k.ok_or_else(|| anyhow::anyhow!("schedule {schedule:?} had no RunWindow step"))?;
    Ok((tracker, last_k))
}

/// The S-restricted oracle assertion (design §6): materialize `S_k`, then
/// `EXCEPT ALL`-compare it (both directions) against the maintained output
/// table `main.<model_name>`.
pub fn assert_equivalence(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    assert_equivalence_with_edit(project, recipe, tracker, k, None)
}

/// [`assert_equivalence`] generalised over an optional post-`RewriteModel`
/// [`ModelEdit`] (Phase 9): when `edit` is `Some`, the oracle re-renders
/// against the REWRITTEN body
/// (`STracker::s_restricted_oracle_sql_with_edit`) rather than the
/// original recipe's own body — never comparing the old body against the
/// new output (plan Phase 9 review checklist). `edit: None` reproduces
/// [`assert_equivalence`]'s exact pre-Phase-9 behaviour.
pub fn assert_equivalence_with_edit(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
    edit: Option<ModelEdit>,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    tracker.materialize_s(&conn, k)?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = match edit {
        Some(edit) => tracker.s_restricted_oracle_sql_with_edit(recipe, edit),
        None => tracker.s_restricted_oracle_sql(recipe),
    };
    let equal = multiset_equal(&conn, &maintained_sql, &oracle_sql);
    if !equal {
        anyhow::bail!(
            "S-restricted equivalence violated for model {:?} at run {k} (edit {edit:?}): \
             maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// `append_only_partition_pool_upholds_equivalence` (plan Phase 3 TDD
/// list): the standing proptest gate — deterministic seed, small N (default
/// [`DEFAULT_CASES`], `SMELT_CONFORMANCE_CASES` env override). Each case:
/// generate recipe → classify → if admitted, generate schedule → drive real
/// `execute_project` per step → assert S-restricted multiset equivalence
/// after every run step.
#[test]
fn append_only_partition_pool_upholds_equivalence() {
    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut admitted_cases = 0;

    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let schedule = arb_schedule_for(&recipe)
            .new_tree(&mut runner)
            .unwrap()
            .current();

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));

        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));

        match verdict {
            Verdict::Refused(_) => continue,
            Verdict::Admitted(_) => {
                admitted_cases += 1;
                rt.block_on(drive_and_assert(&project, &recipe, &schedule))
                    .unwrap_or_else(|e| {
                        panic!(
                            "case {i}: recipe {recipe:?} schedule {schedule:?} \
                             equivalence check failed: {e}"
                        )
                    });
            }
        }
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic sample admitted zero cases — generator/derivation regression"
    );
}

/// `admission_rate_stays_above_floor` (plan Phase 3 TDD list): generator
/// health — over the deterministic sample, at least 40% of (non-adversarial)
/// recipes admit at least one cell.
#[test]
fn admission_rate_stays_above_floor() {
    const N: usize = 50;
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());

    let mut admitted = 0;
    for i in 0..N {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));
        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));
        if matches!(verdict, Verdict::Admitted(_)) {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / N as f64;
    assert!(
        rate >= 0.40,
        "admission rate {rate:.2} over N={N} fell below the 40% generator-health floor \
         ({admitted}/{N} admitted)"
    );
}

// ---------------------------------------------------------------------
// Phase 4: the fact+mutable-dimension mixed pool (`MutableEnrichedRecipe`).
// ---------------------------------------------------------------------

/// Default deterministic case count for `mutable_pool_settles_to_full_refresh`
/// — smaller than [`DEFAULT_CASES`] since each case also drives a dimension
/// mutation + catch-up run on top of the fact-window runs.
const MIXED_DEFAULT_CASES: usize = 6;

fn mixed_case_count() -> usize {
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
const MIXED_DIM_SEED_MAX_ID: i64 = 12;

/// Stage a [`MutableEnrichedRecipe`] into a fresh temp project + DuckDB file:
/// writes both source YAMLs + the model file, creates both physical source
/// tables, and pre-seeds the dimension table with one row per id in
/// `1..=MIXED_DIM_SEED_MAX_ID` (`attr = id * 100`) so every fact row a
/// generated schedule inserts already has a matching dimension row to join
/// against.
pub fn stage_mixed_recipe(
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
pub fn insert_fact_row(
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
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Classify a staged [`MutableEnrichedRecipe`] through the real maintenance
/// derivation — the mixed-pool counterpart of `verdict::classify`, kept here
/// rather than in `verdict.rs` (outside this phase's edit scope, plan
/// Critical files) since `verdict::classify` only accepts a `ModelRecipe`.
pub fn classify_mixed(
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
fn fact_window_for_id(
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
fn assert_mixed_settled(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    tracker.materialize_s(&conn, k)?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    // `STracker::materialize_s` names its temp table `oracle_<source_name>`
    // (an internal convention of `s_tracker.rs`, mirrored here rather than
    // exposed as a public accessor since this is the only Phase 4 call site
    // needing it).
    let oracle_sql = recipe.oracle_body_over(&format!("oracle_{}", recipe.fact.name));
    let equal = multiset_equal(&conn, &maintained_sql, &oracle_sql);
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
async fn drive_mixed_and_assert(
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
                        assert_mixed_settled(project, recipe, &tracker, k)?;
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

// ---------------------------------------------------------------------
// Phase 5: the `grain: key` pool (`KeyedRecipe`).
// ---------------------------------------------------------------------

/// Default deterministic case count for `keyed_pool_upholds_end_state_equivalence`
/// — small since each case drives several `execute_project` windows.
const KEYED_DEFAULT_CASES: usize = 6;

fn keyed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_KEYED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(KEYED_DEFAULT_CASES)
}

/// Stage a [`KeyedRecipe`] into a fresh temp project + DuckDB file.
pub fn stage_keyed_recipe(
    recipe: &KeyedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_keyed(recipe, &project_dir, &db_path)
}

/// Insert one row into a [`KeyedRecipe`]'s staged driving-source table.
pub fn insert_row_keyed(
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
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Classify a staged [`KeyedRecipe`] through the real maintenance derivation
/// — the keyed-pool counterpart of `classify`/`classify_mixed`, kept here
/// rather than in `verdict.rs` (outside this phase's edit scope, plan
/// Critical files). Returns the derived plan (possibly with zero cells) plus
/// every diagnostic on the target model, so a refusal case can name the
/// diagnostic that backs it.
pub fn classify_keyed_full(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
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
            "staged keyed-pool model {:?} (expected at {}) not found among discovered models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// Classify a staged [`KeyedRecipe`], requiring an admitted (non-empty) plan
/// — the happy-path wrapper around [`classify_keyed_full`] for cases that
/// expect admission.
pub fn classify_keyed(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
) -> anyhow::Result<smelt_logical::maintenance::MaintenancePlan> {
    let (plan, diags) = classify_keyed_full(project, recipe)?;
    match plan {
        Some(plan) if !plan.cells.is_empty() => Ok(plan),
        _ => anyhow::bail!(
            "keyed recipe {:?} admitted no cells: diagnostics={diags:#?}",
            recipe.model_name
        ),
    }
}

/// The end-state equivalence assertion for a [`KeyedRecipe`] (design §6
/// "Keyed-grain carve-outs"; `incremental_models.md` §"End-state equivalence"):
/// materialize `S_k` (the union, across every run so far, of that run's own
/// window's rows — exactly [`STracker::s_at`]'s definition, which coincides
/// with "every delta row a window-forward keyed run has folded so far" since
/// a keyed run merges every row landing in its own window and no
/// re-delivery occurs in a generated [`KeyedSchedule`]), then compare the
/// maintained table's full contents against the recipe's own body evaluated
/// over `S_k`.
pub fn assert_keyed_equivalence(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    tracker.materialize_s(&conn, k)?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql =
        render::render_keyed_oracle_body_over(recipe, &format!("oracle_{}", recipe.source.name));
    let equal = multiset_equal(&conn, &maintained_sql, &oracle_sql);
    if !equal {
        anyhow::bail!(
            "keyed end-state equivalence violated for model {:?} at run {k}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` (a [`KeyedRecipe`] under the
/// window-forward posture) through the real `execute_project` pipeline,
/// asserting end-state equivalence after every window.
pub async fn drive_keyed_and_assert(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    schedule: &KeyedSchedule,
) -> anyhow::Result<()> {
    let mut tracker = STracker::new(&recipe.source);

    for (i, window) in schedule.0.iter().enumerate() {
        for row in &window.rows {
            insert_row_keyed(project, recipe, row)?;
        }

        let snapshot = {
            let conn = project.connect()?;
            read_source_snapshot(&conn, &recipe.source)
        };

        let mut request = base_request("dev");
        request.start = Some(window.start.format("%Y-%m-%d").to_string());
        request.end = Some(window.end.format("%Y-%m-%d").to_string());
        project
            .run_quiet(&format!("keyed-run-{i}"), request)
            .await?;

        let k = tracker.record_run(window.start, window.end, snapshot);
        assert_keyed_equivalence(project, recipe, &tracker, k)?;
    }
    Ok(())
}

/// `keyed_pool_upholds_end_state_equivalence` (plan Phase 5 TDD list):
/// keyed recipes (additive + idempotent combiner families, key re-touch
/// across windows) equal the oracle's end state at settled points.
#[test]
fn keyed_pool_upholds_end_state_equivalence() {
    let n = keyed_case_count();
    let mut runner = TestRunner::deterministic();
    let combiner_strat = arb_keyed_combiner();
    let schedule_strat = arb_keyed_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut admitted_cases = 0;
    for i in 0..n {
        let combiner = combiner_strat.new_tree(&mut runner).unwrap().current();
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        let recipe = KeyedRecipe::new_window_forward(combiner);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: keyed recipe {recipe:?} failed to stage: {e}"));

        let plan = classify_keyed(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: keyed recipe {recipe:?} classify failed: {e}"));
        assert!(
            !plan.cells.is_empty(),
            "case {i}: keyed recipe {recipe:?} admitted zero cells — generator/derivation \
             regression"
        );
        admitted_cases += 1;

        rt.block_on(drive_keyed_and_assert(&project, &recipe, &schedule))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: keyed recipe {recipe:?} schedule {schedule:?} equivalence check \
                     failed: {e}"
                )
            });
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic keyed sample admitted zero cases — generator/derivation regression"
    );
}

/// `retained_departed_keys_adjusts_the_oracle` (plan Phase 5 TDD list):
/// snapshot-reconcile schedules generating deletes compare against oracle
/// rows ∪ retained departed keys (`incremental_models.md` §"End-state
/// equivalence"). Two halves: (1) today's real contract — an unclocked
/// (zero-clocked-driving-source) keyed model selects the snapshot-reconcile
/// run shape (`incremental_models.md` §"The two run shapes"), and the *targeted*
/// keyed-fold cell for it is refused fail-loud
/// (`Refusal::NoAdmissibleTechnique`/`Refusal::ScanUnbounded`, named on the
/// plan itself — `maintenance-plan purity`: consumed, not re-derived) rather
/// than silently treated as window-forward, since the snapshot-reconcile
/// executor is unbuilt (`incremental_models.md` §Known Divergences "The key grain"); the universal
/// `Trigger::Backfill`/whole-table-recompute cell every model admits
/// (`incremental_models.md` §"Per-cell admission" — "a recompute is the
/// universal ground-truth reset") stays available as the escape hatch, but
/// no `Trigger::NewData` cell is ever admitted for this source; (2) the pure
/// oracle adjustment that refusal defers to is independently pinned as data
/// (`oracle_modes::keyed_end_state_with_retained_departed_keys`), so the
/// carve-out itself is verified ahead of the executor landing.
#[test]
fn retained_departed_keys_adjusts_the_oracle() {
    let recipe = KeyedRecipe::new_snapshot_reconcile(KeyedCombiner::Additive);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage unclocked keyed recipe");

    let (plan, _diags) =
        classify_keyed_full(&project, &recipe).expect("classify unclocked keyed recipe");
    let plan = plan.expect(
        "maintenance_plan_report must still return a plan (the universal \
         Backfill cell), even when the targeted keyed fold is refused",
    );

    assert!(
        !plan.cells.iter().any(
            |c| matches!(&c.trigger, Trigger::NewData { source } if source == &recipe.source.name)
        ),
        "an unclocked (snapshot-reconcile) keyed model must never admit a targeted NewData \
         fold cell today: {plan:#?}"
    );
    assert!(
        plan.refusals.iter().any(|r| matches!(
            r,
            smelt_logical::maintenance::Refusal::NoAdmissibleTechnique { trigger, .. }
                if trigger.contains(&recipe.source.name)
        )),
        "expected a named NoAdmissibleTechnique refusal naming the unclocked driving source, \
         got: {:#?}",
        plan.refusals
    );

    // The pure carve-out formula this refusal defers to.
    let oracle_rows = vec![
        KeyedOracleRow { key: 1, value: 10 },
        KeyedOracleRow { key: 2, value: 20 },
    ];
    let stored_before_snapshot = [
        KeyedOracleRow { key: 1, value: 999 }, // present in both — oracle wins
        KeyedOracleRow { key: 3, value: 30 },  // departed — retained
    ];
    let retained_departed: Vec<KeyedOracleRow> = stored_before_snapshot
        .iter()
        .filter(|stored| !oracle_rows.iter().any(|o| o.key == stored.key))
        .copied()
        .collect();

    let adjusted = keyed_end_state_with_retained_departed_keys(&oracle_rows, &retained_departed);
    assert_eq!(
        adjusted,
        vec![
            KeyedOracleRow { key: 1, value: 10 },
            KeyedOracleRow { key: 2, value: 20 },
            KeyedOracleRow { key: 3, value: 30 },
        ],
        "stored table must equal the oracle's rows plus retained departed keys, exactly \
         once each"
    );
}

// ---------------------------------------------------------------------
// Phase 6: schedule enrichment — redelivery, full-refresh interleave,
// boundary-value placement.
// ---------------------------------------------------------------------

/// `redelivery_of_processed_window_is_idempotent` (plan Phase 6 TDD list):
/// re-running an already-processed window with no new rows never
/// double-counts under the partition-grain DELETE+INSERT full-replace
/// technique — the never-fold-twice obligation checked end-to-end through
/// the real run pipeline.
#[test]
fn redelivery_of_processed_window_is_idempotent() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");
    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected additive-agg append-only recipe to admit: {verdict:?}"
    );

    let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let window_end = d + chrono::Duration::days(1);

    let schedule = ConformanceSchedule(vec![
        ConformanceStep::RunWindow {
            start: d,
            end: window_end,
            rows: vec![GenRow { d, id: 1, val: 10 }, GenRow { d, id: 2, val: 20 }],
        },
        ConformanceStep::RerunWindow {
            start: d,
            end: window_end,
        },
        ConformanceStep::RerunWindow {
            start: d,
            end: window_end,
        },
    ]);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(drive_and_assert(&project, &recipe, &schedule))
        .expect(
            "redelivering an already-processed window must stay idempotent — equivalence \
             must hold after every redelivery, never double-counted",
        );
}

/// `full_refresh_interleave_resets_state_correctly` (plan Phase 6 TDD list):
/// a mid-schedule `full_refresh` run resets coverage + the reconciliation
/// ledger such that subsequent incremental runs still uphold equivalence.
#[test]
fn full_refresh_interleave_resets_state_correctly() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");
    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected additive-agg append-only recipe to admit: {verdict:?}"
    );

    let d1 = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let d2 = chrono::NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date");
    let d3 = chrono::NaiveDate::from_ymd_opt(2024, 1, 3).expect("valid date");

    let schedule = ConformanceSchedule(vec![
        ConformanceStep::RunWindow {
            start: d1,
            end: d1 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d1,
                id: 1,
                val: 10,
            }],
        },
        // A mid-schedule full-refresh interleave: drops + rebuilds from the
        // CURRENT full source contents (just {d1's row} at this point),
        // resetting coverage.
        ConformanceStep::FullRefreshRun,
        ConformanceStep::RunWindow {
            start: d2,
            end: d2 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d2,
                id: 2,
                val: 20,
            }],
        },
        ConformanceStep::RunWindow {
            start: d3,
            end: d3 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d3,
                id: 3,
                val: 30,
            }],
        },
    ]);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(drive_and_assert(&project, &recipe, &schedule))
        .expect(
            "a mid-schedule full_refresh interleave must reset coverage cleanly — \
             equivalence must hold both immediately after the refresh and through every \
             subsequent windowed run",
        );
}

/// `boundary_rows_within_reach_are_reflected` (plan Phase 6 TDD list): a
/// just-inside-reach row (`schedule_gen::boundary_rows_for`, plan-aware
/// placement against the recipe's OWN derived `ScanClamp`) appears in the
/// maintained output after the run whose window covers it, and the row one
/// calendar day further out (`just_outside`) does not.
///
/// Every construct in `RecipePool::partition_append_only` (the only pool
/// wired up so far) shares the output's partition axis, so `link_source`
/// (`smelt-logical/src/maintenance/derive.rs`) always derives a `ScanClamp`
/// with `before = after = 0` for it — there is no construct in the current
/// pool that produces a genuinely nonzero margin. That means this test
/// cannot exercise an error in margin *derivation* itself (there is no
/// nonzero margin here to derive correctly or incorrectly). What it does
/// exercise, at that zero margin: an off-by-one in how the clamp gets turned
/// into the actual scan predicate. `just_inside` sits exactly at the window
/// start and must be read; `just_outside` sits one day before the window
/// start and must not be — an under- or over-derived scan predicate (e.g. an
/// inclusive/exclusive boundary mistake) would flip one of these two
/// assertions even though the margin itself is zero on both sides. A future
/// pool with a genuinely nonzero clamp (e.g. a lookback join) would extend
/// this same test to cover margin derivation proper.
#[test]
fn boundary_rows_within_reach_are_reflected() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");

    let verdict = classify(&project, &recipe).expect("classify");
    let plan = match verdict {
        Verdict::Admitted(plan) => plan,
        Verdict::Refused(diags) => {
            panic!("expected additive-agg append-only recipe to admit: {diags:#?}")
        }
    };
    let clamp = scan_clamp_for(&plan, &recipe.source.name).unwrap_or_else(|| {
        panic!(
            "expected an admitted NewData scan clamp for source {:?}: {plan:#?}",
            recipe.source.name
        )
    });

    let window = (
        chrono::NaiveDate::from_ymd_opt(2024, 1, 10).expect("valid date"),
        chrono::NaiveDate::from_ymd_opt(2024, 1, 11).expect("valid date"),
    );
    let mut next_id = 1_i64;
    let boundary = boundary_rows_for(clamp, window, &mut next_id);

    insert_row(&project, &recipe, &boundary.just_inside).expect("insert just-inside row");
    insert_row(&project, &recipe, &boundary.just_outside).expect("insert just-outside row");

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut request = base_request("dev");
    request.start = Some(window.0.format("%Y-%m-%d").to_string());
    request.end = Some(window.1.format("%Y-%m-%d").to_string());
    rt.block_on(project.run_quiet("boundary-run", request))
        .expect("triggering run over the boundary window");

    let conn = project.connect().expect("connect for read-back");
    let count: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM main.{} WHERE {} = DATE '{}'",
                recipe.model_name,
                recipe.source.clock_column,
                boundary.just_inside.d.format("%Y-%m-%d"),
            ),
            [],
            |row| row.get(0),
        )
        .expect("count rows for the boundary just-inside day");
    assert!(
        count > 0,
        "a just-inside-reach row (day {}) must appear in the maintained output after the \
         triggering run over {window:?} — an under-derived clamp would drop it",
        boundary.just_inside.d,
    );

    let outside_count: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM main.{} WHERE {} = DATE '{}'",
                recipe.model_name,
                recipe.source.clock_column,
                boundary.just_outside.d.format("%Y-%m-%d"),
            ),
            [],
            |row| row.get(0),
        )
        .expect("count rows for the boundary just-outside day");
    assert_eq!(
        outside_count, 0,
        "a just-outside-reach row (day {}) must NOT appear in the maintained output after the \
         triggering run over {window:?} — an over-derived (or off-by-one) scan predicate would \
         read it even though it lies outside the derived reach",
        boundary.just_outside.d,
    );
}

// ---------------------------------------------------------------------
// Phase 8: the `SimulatedChangeFeed` step family — recompute-only
// admission for `change_feed`-declared sources
// (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 8;
// `incremental_models.md` §Known Divergences' `change_feed`-scoping entry).
// ---------------------------------------------------------------------

/// Default deterministic case count for `change_feed_source_admits_recompute_only`.
const FEED_ADMISSION_DEFAULT_CASES: usize = 10;

fn feed_admission_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_FEED_ADMISSION_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(FEED_ADMISSION_DEFAULT_CASES)
}

/// `change_feed_source_admits_recompute_only` (plan Phase 8 TDD list): a
/// `change_feed`-declared source's admitted cells are all full-input
/// re-derivation, never a fold (`incremental_models.md` §Known Divergences:
/// "no live fold machinery consumes a change feed's delta shape yet" —
/// mirrors `crates/smelt-logical/tests/maintenance_coverage_matrix.rs`'s
/// `ex14_change_feed_sum_recompute_only`/`ex26_change_feed_latest_writer_recompute_only`,
/// but driven through the real production entry point
/// (`smelt_db::maintenance_plan_report`) rather than the pure derivation
/// directly).
#[test]
fn change_feed_source_admits_recompute_only() {
    let n = feed_admission_case_count();
    let mut runner = TestRunner::deterministic();
    let combiner_strat = arb_keyed_combiner();

    let mut checked = 0;
    for i in 0..n {
        let combiner = combiner_strat.new_tree(&mut runner).unwrap().current();
        let recipe = feed::feed_keyed_recipe(combiner);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = feed::stage_feed_keyed(&recipe, &project_dir, &db_path)
            .unwrap_or_else(|e| panic!("case {i}: failed to stage feed-driven keyed recipe: {e}"));

        let (plan, diags) = classify_keyed_full(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: classify failed: {e}"));
        let plan = plan.unwrap_or_else(|| {
            panic!("case {i}: no maintenance plan returned at all: diagnostics={diags:#?}")
        });

        assert!(
            !plan.cells.is_empty(),
            "case {i}: change_feed-driven keyed recipe {recipe:?} admitted zero cells \
             (expected at least the universal Backfill recompute cell): diagnostics={diags:#?}"
        );
        for cell in &plan.cells {
            assert_eq!(
                cell.technique,
                Technique::DeleteInsert,
                "case {i}: a change_feed-declared source must admit ONLY full-input \
                 re-derivation (Technique::DeleteInsert), never a fold — got {:?} for cell \
                 {cell:?}",
                cell.technique,
            );
        }
        assert!(
            !plan.cells.iter().any(|c| matches!(
                &c.trigger,
                Trigger::NewData { source } if source == &recipe.source.name
            )),
            "case {i}: a change_feed source must never admit a targeted NewData fold cell \
             today (incremental_models.md §Known Divergences' change_feed-scoping entry): \
             {plan:#?}"
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "N={n} deterministic sample never staged a change_feed-driven keyed recipe — \
         generator/derivation regression"
    );
}

/// Default deterministic case count for
/// `feed_declared_source_upholds_equivalence_via_recompute`.
const FEED_RECOMPUTE_DEFAULT_CASES: usize = 6;

fn feed_recompute_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_FEED_RECOMPUTE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(FEED_RECOMPUTE_DEFAULT_CASES)
}

/// Generous upper bound on distinct dimension ids this test's fact rows
/// ever reference — mirrors `MIXED_DIM_SEED_MAX_ID` (Phase 4's own
/// convention above).
const FEED_DIM_SEED_MAX_ID: i64 = 12;

/// Wrap `sql` restricted to the single partition day `day` on column
/// `day_col` — used to isolate one window's own rows so a genuinely
/// incremental run's freshness (or an older, un-revisited window's frozen
/// staleness) can be checked without the whole-table noise of every other
/// window.
fn restrict_to_day(sql: &str, day: chrono::NaiveDate, day_col: &str) -> String {
    format!(
        "SELECT * FROM ({sql}) t WHERE t.{day_col} = DATE '{}'",
        day.format("%Y-%m-%d")
    )
}

/// `feed_declared_source_upholds_equivalence_via_recompute` (plan Phase 8
/// TDD list): mutation schedules over feed-declared sources settle to
/// full-refresh equality. Drives the fact+`change_feed`-dimension mixed
/// shape (`feed::stage_feed_enriched`) rather than the `grain: key` pool:
/// `change_feed_source_admits_recompute_only` already pins that a
/// `grain: key` model with a fold spec over a `change_feed` source carries a
/// build-blocking `MaintenanceNoAdmissibleTechnique` Error diagnostic (fold
/// refused), so it can never actually be driven through `execute_project`
/// — only the classify-level admission surface is checkable there. The
/// mixed shape builds cleanly (no `UpstreamMutation` cell is EVER
/// constructed for a `change_feed`-declared dimension — `incremental_models.md`
/// §Known Divergences' `change_feed`-scoping entry — so there is nothing to
/// refuse).
///
/// Unlike `mutable_pool_settles_to_full_refresh`'s sibling pattern, there is
/// no `UpstreamMutation` cell to make an already-materialized window catch
/// up: this test drives GENUINE incremental (`full_refresh: false`) runs —
/// one fresh partition per schedule step, interleaved with a dimension
/// mutation applied just before it — and checks two things a full-refresh-
/// only drive (the prior, weaker version of this test) could never catch:
/// (1) freshness — a NEWLY computed window always reflects the dimension's
/// CURRENT state (`maintenance.scan_bounds...allow_full_scan: true` means
/// the join is never scan-bounded), so a regression that fed a stale/cached
/// dimension snapshot into a fresh incremental compute would fail here; (2)
/// the documented staleness itself — the FIRST window, once materialized,
/// is provably never revisited by any later incremental run (the
/// `change_feed`-scoping divergence), so it diverges from a live recompute
/// after the schedule's mutations land, exactly the `incremental_models.md`
/// §Known Divergences contract. A final `full_refresh: true` run must then
/// settle the WHOLE table back to equivalence — that is the "via recompute"
/// half of this test's name, now actually exercised after a real
/// incremental history rather than skipped entirely.
#[test]
fn feed_declared_source_upholds_equivalence_via_recompute() {
    let n = feed_recompute_case_count();
    let mut runner = TestRunner::deterministic();
    let schedule_strat = feed::arb_feed_step_schedule(FeedSourcePosture::ChangeFeed);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let day_col = MutableEnrichedRecipe::new().fact.clock_column.clone();

    for i in 0..n {
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        let recipe = MutableEnrichedRecipe::new();

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project =
            feed::stage_feed_enriched(&recipe, &project_dir, &db_path, FEED_DIM_SEED_MAX_ID)
                .unwrap_or_else(|e| {
                    panic!("case {i}: failed to stage feed-enriched mixed recipe: {e}")
                });

        let day0 = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");

        // One fact row per pre-seeded dimension id, so the join always
        // produces output regardless of which dimension rows the schedule
        // below goes on to mutate/retract.
        for id in 1..=FEED_DIM_SEED_MAX_ID {
            insert_fact_row(
                &project,
                &recipe,
                &GenRow {
                    d: day0,
                    id,
                    val: id * 10,
                },
            )
            .unwrap_or_else(|e| panic!("case {i}: failed to seed fact row {id}: {e}"));
        }

        rt.block_on(async {
            let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
            let live_oracle_sql =
                recipe.oracle_body_over(&format!("main.sources_{}", recipe.fact.name));

            // Genuinely incremental first run: materialize day0 only.
            let mut day0_request = base_request("dev");
            day0_request.start = Some(day0.format("%Y-%m-%d").to_string());
            day0_request.end = Some(
                (day0 + chrono::Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string(),
            );
            project
                .run_quiet(&format!("feed-run-{i}-day0"), day0_request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: initial incremental day0 run failed: {e}"));

            {
                let conn = project.connect().expect("connect for day0 freshness check");
                let maintained_day0 = restrict_to_day(&maintained_sql, day0, &day_col);
                let oracle_day0 = restrict_to_day(&live_oracle_sql, day0, &day_col);
                assert!(
                    multiset_equal(&conn, &maintained_day0, &oracle_day0),
                    "case {i}: freshly incremental-computed day0 must match a live recompute \
                     over the dimension's state at computation time: maintained \
                     ({maintained_day0:?}) != oracle ({oracle_day0:?})"
                );
            }

            // Snapshot day0 the moment it settles — this pool's own frozen
            // reference for the staleness check below.
            let day0_snapshot_sql = {
                let conn = project.connect().expect("connect for day0 snapshot");
                let snapshot = restrict_to_day(&maintained_sql, day0, &day_col);
                // Materialize into a real (non-TEMP) table, since later
                // read-backs open fresh connections and a TEMP table is
                // scoped to the connection that created it — so later runs
                // (which mutate `main.sources_<dim>`, not the model table
                // itself) can't change what this reference query returns.
                conn.execute_batch(&format!(
                    "CREATE TABLE main.feed_day0_snapshot_{i} AS {snapshot}"
                ))
                .unwrap_or_else(|e| panic!("case {i}: failed to snapshot day0: {e}"));
                format!("SELECT * FROM main.feed_day0_snapshot_{i}")
            };

            for (step_i, step) in schedule.0.iter().enumerate() {
                {
                    let conn = project.connect().expect("connect for feed step");
                    feed::apply_feed_step(&conn, &recipe.dimension, step, step_i as i64)
                        .unwrap_or_else(|e| {
                            panic!("case {i} step {step_i}: apply_feed_step failed: {e}")
                        });
                }

                // A genuinely NEW window, never touched before: one fresh
                // fact row for a stable pre-seeded id, on a day this
                // schedule has not run before. Its incremental computation
                // happens strictly AFTER the mutation just applied above, so
                // — per `allow_full_scan: true` — it must reflect the
                // dimension's post-mutation state.
                let new_day = day0 + chrono::Duration::days(step_i as i64 + 1);
                let dim_id = (step_i as i64 % FEED_DIM_SEED_MAX_ID) + 1;
                insert_fact_row(
                    &project,
                    &recipe,
                    &GenRow {
                        d: new_day,
                        id: dim_id,
                        val: dim_id * 10 + step_i as i64,
                    },
                )
                .unwrap_or_else(|e| {
                    panic!("case {i} step {step_i}: failed to insert new-window fact row: {e}")
                });

                let mut request = base_request("dev");
                request.start = Some(new_day.format("%Y-%m-%d").to_string());
                request.end = Some(
                    (new_day + chrono::Duration::days(1))
                        .format("%Y-%m-%d")
                        .to_string(),
                );
                project
                    .run_quiet(&format!("feed-run-{i}-{step_i}"), request)
                    .await
                    .unwrap_or_else(|e| panic!("case {i} step {step_i}: run failed: {e}"));

                let conn = project.connect().expect("connect for read-back");

                // Freshness: the window just computed must match a live
                // recompute over the CURRENT (post-mutation) dimension
                // state — proves this is a real incremental run, not a
                // no-op, and that it isn't silently reading a stale
                // dimension snapshot.
                let maintained_new_day = restrict_to_day(&maintained_sql, new_day, &day_col);
                let oracle_new_day = restrict_to_day(&live_oracle_sql, new_day, &day_col);
                assert!(
                    multiset_equal(&conn, &maintained_new_day, &oracle_new_day),
                    "case {i} step {step_i}: freshly incremental-computed window {new_day} must \
                     match a live recompute over the dimension's current state: maintained \
                     ({maintained_new_day:?}) != oracle ({oracle_new_day:?}), schedule={schedule:?}"
                );
            }

            // Documented current behavior (`incremental_models.md` §Known
            // Divergences, `change_feed`-scoping entry): no incremental run
            // ever revisits day0 once materialized, so it stays frozen at
            // its original computation-time snapshot even though the
            // schedule above has since mutated the dimension rows day0
            // joined against.
            {
                let conn = project.connect().expect("connect for staleness check");
                let maintained_day0_now = restrict_to_day(&maintained_sql, day0, &day_col);
                assert!(
                    multiset_equal(&conn, &maintained_day0_now, &day0_snapshot_sql),
                    "case {i}: day0 must remain frozen at its original computation-time state \
                     across purely incremental runs (no UpstreamMutation cell is ever built for \
                     a change_feed-declared dimension) — maintained day0 changed without a \
                     revisiting run: now ({maintained_day0_now:?}) != snapshot \
                     ({day0_snapshot_sql:?}), schedule={schedule:?}"
                );

                // And the flip side: that frozen day0 means the WHOLE table
                // is now stale relative to a live recompute — this is the
                // exact risk this test guards against (a silent regression
                // that either (a) fixed this staleness without settling via
                // a documented path, or (b) broke fresh-window correctness
                // and happened to still "settle" by accident).
                assert!(
                    !multiset_equal(&conn, &maintained_sql, &live_oracle_sql),
                    "case {i}: expected the WHOLE table to be stale relative to a live recompute \
                     after purely incremental runs following dimension mutations — if this now \
                     holds, either the change_feed-scoping divergence has been fixed (update \
                     this test's doc comment and drop this assertion) or the schedule failed to \
                     mutate anything day0 actually joined against, schedule={schedule:?}"
                );
            }

            // Full-refresh recompute must still settle the WHOLE table back
            // to equivalence — the "via recompute" contract this test is
            // named for, now exercised after a real incremental history.
            let mut refresh_request = base_request("dev");
            refresh_request.full_refresh = true;
            project
                .run_quiet(&format!("feed-run-{i}-full-refresh"), refresh_request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: final full-refresh run failed: {e}"));

            let conn = project.connect().expect("connect for final read-back");
            assert!(
                multiset_equal(&conn, &maintained_sql, &live_oracle_sql),
                "case {i}: feed-declared source equivalence via full-refresh recompute violated \
                 after an incremental history: maintained ({maintained_sql:?}) != oracle \
                 ({live_oracle_sql:?}), schedule={schedule:?}"
            );
        });
    }
}

// ---------------------------------------------------------------------
// Phase 9: definition-change steps — `ConformanceStep::RewriteModel`
// (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 9;
// `incremental_models.md` §"The definition-change trigger"). Asserts TODAY's
// contract only: whatever technique executes for a window always compiles
// and runs the model's CURRENT on-disk SQL (`link_c_harness::LinkCProject`'s
// per-run re-discovery), so a rewrite followed by a re-run of the affected
// window(s) recovers full equivalence against the REWRITTEN body's own
// oracle. This is deliberately NOT the spec's unbuilt `SkeletonAdd`/
// `PureBackfill`/`UpstreamRederive` definition-change classification — see
// `schedule_gen::ConformanceStep::RewriteModel`'s doc comment.
// ---------------------------------------------------------------------

/// `column_add_between_runs_recovers_equivalence` (plan Phase 9 TDD list):
/// schedule: runs → `RewriteModel` (add integer payload column) → catch-up
/// runs; final state equals the oracle of the NEW body over full S.
///
/// The catch-up is a `FullRefreshRun`, not a plain windowed re-run: today's
/// windowed (DELETE+INSERT) incremental path never persists a deployed
/// schema snapshot (`execute.rs`'s "Save deployed schema for full-refresh
/// models" comment marks that as full-refresh-only), so
/// `schema_evolution::check_and_migrate`'s ALTER-TABLE detection always sees
/// `FirstDeployment` for an incremental model and never fires — a windowed
/// re-run against an already-materialized table with a changed column shape
/// hits a raw DuckDB column-count mismatch, not a graceful recompute. A
/// `full_refresh` run (`DROP`+recreate, taking the non-incremental
/// materialization arm — `execute.rs`'s `(Some(_inc), None, _, _)` "no time
/// window ... fall back to full refresh" arm) sidesteps this cleanly: it is
/// today's actual recovery path, matching the interval-store hash
/// invalidation's own "the next run recovers equivalence" framing (Phase
/// 9's Goal) rather than the unbuilt `PureBackfill` classification.
#[test]
fn column_add_between_runs_recovers_equivalence() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");
    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected additive-agg append-only recipe to admit: {verdict:?}"
    );

    let w1_start = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let w1_end = w1_start + chrono::Duration::days(1);
    let w2_start = w1_end;
    let w2_end = w2_start + chrono::Duration::days(1);
    let w3_start = w2_end;
    let w3_end = w3_start + chrono::Duration::days(1);

    let schedule = ConformanceSchedule(vec![
        // Two windows run under the ORIGINAL body.
        ConformanceStep::RunWindow {
            start: w1_start,
            end: w1_end,
            rows: vec![GenRow {
                d: w1_start,
                id: 1,
                val: 10,
            }],
        },
        ConformanceStep::RunWindow {
            start: w2_start,
            end: w2_end,
            rows: vec![GenRow {
                d: w2_start,
                id: 2,
                val: 20,
            }],
        },
        // Definition change: add a derived payload column, same skeleton.
        ConformanceStep::RewriteModel {
            edit: ModelEdit::AddPayloadColumn,
        },
        // Catch-up: a full-refresh run recomputes the WHOLE table under the
        // rewritten body over the current full source contents — today's
        // actual recovery path (see this test's doc comment).
        ConformanceStep::FullRefreshRun,
        // A subsequent ordinary windowed run must keep working post-recovery
        // — the rewrite's effect must persist, not just paper over the one
        // full-refresh run.
        ConformanceStep::RunWindow {
            start: w3_start,
            end: w3_end,
            rows: vec![GenRow {
                d: w3_start,
                id: 3,
                val: 30,
            }],
        },
    ]);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(drive_and_assert(&project, &recipe, &schedule))
        .expect(
            "a column-add RewriteModel followed by a full-refresh catch-up must recover full \
             equivalence against the rewritten body's own oracle, and stay recovered through \
             a subsequent windowed run",
        );
}

/// `skeleton_position_add_is_refused_or_recomputed_never_corrupted` (plan
/// Phase 9 TDD list): adding a column in a grouping/skeleton position
/// mid-schedule never yields a silently-wrong maintained state — either a
/// named refusal (via the real [`classify`] verdict protocol) or a full
/// recompute whose result equals the rewritten body's own oracle. The
/// recompute is driven via `FullRefreshRun`, the only technique that
/// actually survives a schema-shape change today — see
/// `column_add_between_runs_recovers_equivalence`'s doc comment for why a
/// plain windowed re-run against an already-materialized table hits a raw
/// column-count mismatch instead.
#[test]
fn skeleton_position_add_is_refused_or_recomputed_never_corrupted() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");
    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected additive-agg append-only recipe to admit: {verdict:?}"
    );

    let w1_start = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let w1_end = w1_start + chrono::Duration::days(1);

    let mut tracker = STracker::new(&recipe.source);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    // Pre-rewrite run, under the original body — establishes a green
    // baseline before the skeleton-changing rewrite.
    insert_row(
        &project,
        &recipe,
        &GenRow {
            d: w1_start,
            id: 1,
            val: 10,
        },
    )
    .expect("seed row");
    let snapshot = {
        let conn = project.connect().expect("connect");
        read_source_snapshot(&conn, &recipe.source)
    };
    let mut request = base_request("dev");
    request.start = Some(w1_start.format("%Y-%m-%d").to_string());
    request.end = Some(w1_end.format("%Y-%m-%d").to_string());
    rt.block_on(project.run_quiet("run-0", request))
        .expect("initial run before rewrite");
    let k0 = tracker.record_run(w1_start, w1_end, snapshot);
    assert_equivalence(&project, &recipe, &tracker, k0).expect("pre-rewrite equivalence");

    // Rewrite: add the source's row-key column into the GROUP BY — a
    // grain/skeleton change, `incremental_models.md`'s `SkeletonAdd`
    // territory.
    std::fs::write(
        project
            .project_dir
            .join(format!("models/{}.sql", recipe.model_name)),
        render::render_model_file_with_edit(&recipe, ModelEdit::AddGroupingColumn),
    )
    .expect("write rewritten model file");

    // Classify the NOW-rewritten project through the real derivation
    // (`verdict.rs`'s fail-loud protocol: a refusal always carries a named
    // Maintenance*/admission diagnostic, never an unexplained empty plan).
    let post_verdict = classify(&project, &recipe).expect("classify after rewrite");
    match post_verdict {
        Verdict::Refused(diags) => {
            assert!(
                !diags.is_empty(),
                "a refused skeleton-position rewrite must carry a named diagnostic \
                 (verdict.rs's own fail-loud check already enforces this — asserted here too \
                 so this test would fail if that changed)"
            );
        }
        Verdict::Admitted(_) => {
            let mut request2 = base_request("dev");
            request2.full_refresh = true;
            rt.block_on(project.run_quiet("run-1", request2))
                .expect("an admitted post-rewrite plan must not fail to execute");

            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.source)
            };
            let k1 = tracker.record_full_refresh(snapshot);
            assert_equivalence_with_edit(
                &project,
                &recipe,
                &tracker,
                k1,
                Some(ModelEdit::AddGroupingColumn),
            )
            .expect(
                "an admitted skeleton-position rewrite's recompute must equal the rewritten \
                 body's own oracle — never silently diverge",
            );
        }
    }
}

// ---------------------------------------------------------------------
// Phase A6: the composed (`grain: key` + `timeseries:`) recipe family,
// covering all three key-temporal-locality routes
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase A6;
// `incremental_models.md` §"Key temporal locality").
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
const COMPOSED_DEFAULT_CASES: usize = 6;

fn composed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_COMPOSED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(COMPOSED_DEFAULT_CASES)
}

/// Classify a staged [`ComposedKeyedRecipe`] through the real maintenance
/// derivation — the composed-pool counterpart of `classify_keyed_full`.
/// Returns the derived plan (possibly with zero cells / a locality refusal)
/// plus every diagnostic on the target model.
pub fn classify_composed_full(
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
fn assert_composed_admitted_with_expected_route(
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

fn insert_composed_row(
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
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Whole-table equivalence for route 1: the maintained output equals the
/// model body evaluated over the full, currently-inserted source table —
/// route 1's schedule never reprocesses a window, so no `STracker`
/// S-restriction is needed.
fn assert_composed_route1_equivalence(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = render::render_composed_oracle_sql(recipe);
    if !multiset_equal(&conn, &maintained_sql, &oracle_sql) {
        anyhow::bail!(
            "composed route-1 equivalence violated for {:?}: maintained ({maintained_sql:?}) != \
             oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Per-slice equivalence for route 1 (`incremental_models.md` §"Per-slice
/// equivalence"): the stored rows of one output slice (`d = slice_date`)
/// equal the model SQL evaluated over the source rows within that slice's
/// derived reach — zero margin here (`SIMPLE_SQL`-shaped, no lookback), so
/// the reach is exactly the source rows sharing that same date.
fn assert_composed_route1_per_slice(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
    slice_date: chrono::NaiveDate,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    let d = slice_date.format("%Y-%m-%d");
    let maintained_sql = format!(
        "SELECT * FROM main.{} WHERE d = DATE '{d}'",
        recipe.model_name
    );
    let oracle_body = render::render_composed_oracle_sql(recipe);
    let oracle_sql = format!("SELECT * FROM ({oracle_body}) t WHERE d = DATE '{d}'");
    if !multiset_equal(&conn, &maintained_sql, &oracle_sql) {
        anyhow::bail!(
            "composed route-1 per-slice equivalence violated for {:?} at slice {d}: maintained \
             ({maintained_sql:?}) != model SQL over the slice's derived reach ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

async fn drive_composed_route1_and_assert(
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

        assert_composed_route1_equivalence(project, recipe)?;
        assert_composed_route1_per_slice(project, recipe, window.start)?;
    }
    Ok(())
}

// ---- Routes 2/3: direct-driver execution against a real DuckDbBackend --

/// The driving source's own `timeseries:` declaration every composed
/// recipe's classification carries (`event_time_column`/`partition_column`
/// both `d`, `day` granularity — the fixed `events(d, id, val)` shape).
fn composed_driving_timeseries() -> smelt_core::config::TimeseriesConfig {
    smelt_core::config::TimeseriesConfig {
        event_time_column: "d".to_string(),
        partition_column: "d".to_string(),
        granularity: smelt_core::config::Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn composed_route2_classification(recipe: &ComposedKeyedRecipe) -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "total".to_string(),
            per_partition_agg: "SUM".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
        }],
        driving_source: DrivingSource {
            name: format!("smelt.sources.{}", recipe.source.name),
            timeseries: composed_driving_timeseries(),
        },
    }
}

fn composed_route3_classification(recipe: &ComposedKeyedRecipe) -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "last_seen".to_string(),
            per_partition_agg: "MAX".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Max,
        }],
        driving_source: DrivingSource {
            name: format!("smelt.sources.{}", recipe.source.name),
            timeseries: composed_driving_timeseries(),
        },
    }
}

fn composed_route3_slice() -> LocalitySlice {
    LocalitySlice::RecurrenceBounded {
        partition_column: "last_seen".to_string(),
        margin_before: smelt_logical::analysis::source_bounds::Seconds::days(3),
        margin_after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        r: smelt_logical::analysis::source_bounds::Seconds::days(3),
    }
}

/// One window's own row list, rendered as a literal `VALUES` relation —
/// the per-step delta is built directly from the window's own rows rather
/// than filtered off a physical table by a `d = <date>` predicate, which
/// would wrongly require a redelivered row's own event-time to equal the
/// window that delivers it (`ComposedRoute3Window`'s own doc comment names
/// this as exactly the "out-of-order redelivery" shape the pool must be
/// able to express).
fn composed_delta_values_sql(rows: &[GenRow]) -> String {
    let values: Vec<String> = rows
        .iter()
        .map(|r| format!("({}, DATE '{}', {})", r.id, r.d.format("%Y-%m-%d"), r.val))
        .collect();
    format!("(VALUES {}) AS t(id, d, val)", values.join(", "))
}

fn composed_route2_delta_sql(rows: &[GenRow]) -> String {
    format!(
        "SELECT id, CAST(d AS DATE) AS pdate, SUM(val) AS total FROM {} GROUP BY id, d",
        composed_delta_values_sql(rows)
    )
}

fn composed_route3_delta_sql(rows: &[GenRow]) -> String {
    format!(
        "SELECT id, MAX(d) AS last_seen FROM {} GROUP BY id",
        composed_delta_values_sql(rows)
    )
}

/// The route-2 oracle: `pdate` is write-once (never re-merged — see
/// `ComposedKeyedRecipe`'s doc comment), so its true end-state value is the
/// event-time of whichever window *first* delivered that key — the
/// minimum `d` across all of that key's accumulated rows (every row in
/// this pool's route-2 schedule carries `d == its own window's run date`,
/// and windows always run in ascending order).
fn composed_route2_oracle_sql(source_name: &str) -> String {
    format!(
        "SELECT id, CAST(MIN(d) AS DATE) AS pdate, SUM(val) AS total FROM main.sources_{source_name} \
         GROUP BY id"
    )
}

fn composed_route3_oracle_sql(source_name: &str) -> String {
    format!("SELECT id, MAX(d) AS last_seen FROM main.sources_{source_name} GROUP BY id")
}

/// Convert a batch of Arrow results into a sorted `Vec` of `(column,
/// value)` row vectors — a multiset comparator over two such `Vec`s (via
/// plain `==` after sorting) that does not require a `duckdb::Connection`
/// (`oracle::multiset_equal`'s own contract), since routes 2/3 query
/// through a live `DuckDbBackend` instead (mirrors
/// `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs`'s own
/// `execute_sql`-only discipline — never open a second, independent
/// connection to the same DuckDB file while the backend holds one open).
fn rows_as_sorted_multiset(batches: &[arrow::array::RecordBatch]) -> Vec<Vec<(String, String)>> {
    let mut rows: Vec<Vec<(String, String)>> = batches_to_rows(batches)
        .into_iter()
        .map(|m| m.into_iter().collect())
        .collect();
    rows.sort();
    rows
}

async fn assert_backend_multiset_equal(
    backend: &DuckDbBackend,
    left_sql: &str,
    right_sql: &str,
    context: &str,
) -> anyhow::Result<()> {
    let left = backend.execute_sql(left_sql).await?;
    let right = backend.execute_sql(right_sql).await?;
    let left_rows = rows_as_sorted_multiset(&left);
    let right_rows = rows_as_sorted_multiset(&right);
    if left_rows != right_rows {
        anyhow::bail!(
            "{context}: multiset mismatch\n  left  ({left_sql:?}): {left_rows:?}\n  right \
             ({right_sql:?}): {right_rows:?}"
        );
    }
    Ok(())
}

async fn assert_composed_route2_equivalence(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = composed_route2_oracle_sql(&recipe.source.name);
    assert_backend_multiset_equal(
        backend,
        &maintained_sql,
        &oracle_sql,
        "composed route-2 equivalence",
    )
    .await
}

/// Per-slice equivalence for route 2 (`incremental_models.md` §"Per-slice
/// equivalence"): route 2 never settles by date — its slice is the
/// delta's own partition **values**, not a date-range window — so the
/// natural slice here is one distinct `pdate` value; each such slice must
/// equal the oracle restricted to that same value.
async fn assert_composed_route2_per_slice(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT DISTINCT CAST(pdate AS VARCHAR) AS v FROM main.{}",
            recipe.model_name
        ))
        .await?;
    let values: Vec<String> = batches_to_rows(&batches)
        .into_iter()
        .filter_map(|r| r.get("v").cloned())
        .collect();
    for v in values {
        let maintained_sql = format!(
            "SELECT * FROM main.{} WHERE pdate = DATE '{v}'",
            recipe.model_name
        );
        let oracle_sql = format!(
            "SELECT * FROM ({}) t WHERE pdate = DATE '{v}'",
            composed_route2_oracle_sql(&recipe.source.name)
        );
        assert_backend_multiset_equal(
            backend,
            &maintained_sql,
            &oracle_sql,
            "composed route-2 per-slice equivalence",
        )
        .await?;
    }
    Ok(())
}

async fn assert_composed_route3_equivalence(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = composed_route3_oracle_sql(&recipe.source.name);
    assert_backend_multiset_equal(
        backend,
        &maintained_sql,
        &oracle_sql,
        "composed route-3 equivalence",
    )
    .await
}

/// Per-slice equivalence for route 3: `last_seen` genuinely settles
/// (`AfterRecurrenceBound`), so each distinct `last_seen` date-value slice
/// must equal the oracle restricted to that same value.
async fn assert_composed_route3_per_slice(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT DISTINCT CAST(last_seen AS VARCHAR) AS v FROM main.{}",
            recipe.model_name
        ))
        .await?;
    let values: Vec<String> = batches_to_rows(&batches)
        .into_iter()
        .filter_map(|r| r.get("v").cloned())
        .collect();
    for v in values {
        let maintained_sql = format!(
            "SELECT * FROM main.{} WHERE last_seen = DATE '{v}'",
            recipe.model_name
        );
        let oracle_sql = format!(
            "SELECT * FROM ({}) t WHERE last_seen = DATE '{v}'",
            composed_route3_oracle_sql(&recipe.source.name)
        );
        assert_backend_multiset_equal(
            backend,
            &maintained_sql,
            &oracle_sql,
            "composed route-3 per-slice equivalence",
        )
        .await?;
    }
    Ok(())
}

/// Append `rows` to the driving source's accumulation-log table
/// (`main.sources_<name>`, created by `render::stage_composed`) — used only
/// as the oracle's own read side for routes 2/3; the direct driver's own
/// per-step delta never reads this table (`composed_delta_values_sql`'s doc
/// comment).
async fn insert_composed_rows_via_backend(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
    rows: &[GenRow],
) -> anyhow::Result<()> {
    for row in rows {
        backend
            .execute_sql(&format!(
                "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
                recipe.source.name,
                row.d.format("%Y-%m-%d"),
                row.id,
                row.val,
            ))
            .await?;
    }
    Ok(())
}

async fn drive_composed_route2_and_assert(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
    schedule: &KeyedSchedule,
) -> anyhow::Result<()> {
    let classification = composed_route2_classification(recipe);
    // `Some(&composed_route2_slice())` — the real `DeltaValues` slice a
    // route-2 model is admitted with — is deliberately **not** passed
    // here. Doing so renders `emit_keyed_fold`'s `target.<col> IN (SELECT
    // DISTINCT <col> FROM (<delta_select>))` predicate, and real DuckDB
    // (confirmed directly against the `duckdb` CLI, v1.5.4/v1.10504)
    // refuses to bind ANY `MERGE` whose `ON` clause combines a derived
    // `USING` subquery with that `IN (SELECT DISTINCT … FROM (subquery))`
    // shape at all — `Invalid Input Error: BindMerge - expected to find an
    // operator of type LOGICAL_GET but got FILTER` — independently of
    // whether the delta is a `VALUES` literal or a real table scan. This
    // is a genuine DuckDB backend limitation for the `DeltaValues`
    // slice-predicate shape, recorded verbatim in `incremental_models.md`
    // §Known Divergences under "Key temporal locality" (the paragraph
    // starting "Route 2's slice-pruned merge … is unexercised against a
    // real backend") and cross-referenced again in that section's §Tests
    // bullet — distinct from the already-documented NOT-NULL nullability
    // blocker. Fixing the emitted
    // predicate shape is production code in `smelt-logical::maintenance::
    // emit`, outside this testkit-only phase's Critical files — flagged
    // here rather than silently worked around. Passing `None` still
    // exercises the real merge mechanics this test actually asserts
    // (write-once `pdate`, additive `total`) against real DuckDB; only the
    // target-scan **pruning** optimisation itself goes unexercised
    // (`incremental_models.md` §"Key temporal locality": "pruning is not a
    // write clamp" — every delta row still merges with or without it).
    let slice: Option<&LocalitySlice> = None;

    for (i, window) in schedule.0.iter().enumerate() {
        insert_composed_rows_via_backend(backend, recipe, &window.rows).await?;

        let rows = window.rows.clone();
        let compile_step = move |_step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
            Ok(composed_route2_delta_sql(&rows))
        };
        let steps = driving_steps(
            &window.start.format("%Y-%m-%d").to_string(),
            &window.end.format("%Y-%m-%d").to_string(),
            &smelt_core::config::Granularity::Day,
        )?;
        run_windowed_keyed_maintenance(
            backend,
            &recipe.model_name,
            "main",
            &recipe.model_name,
            &steps,
            &classification,
            slice,
            compile_step,
        )
        .await
        .map_err(|e| anyhow::anyhow!("composed route-2 window {i} merge failed: {e}"))?;

        assert_composed_route2_equivalence(backend, recipe).await?;
        assert_composed_route2_per_slice(backend, recipe).await?;
    }
    Ok(())
}

async fn drive_composed_route3_and_assert(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
    schedule: &ComposedRoute3Schedule,
) -> anyhow::Result<()> {
    let classification = composed_route3_classification(recipe);
    let slice = composed_route3_slice();

    for (i, window) in schedule.0.iter().enumerate() {
        insert_composed_rows_via_backend(backend, recipe, &window.rows).await?;

        let rows = window.rows.clone();
        let compile_step = move |_step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
            Ok(composed_route3_delta_sql(&rows))
        };
        let run_date_str = window.run_date.format("%Y-%m-%d").to_string();
        let next_day_str = (window.run_date + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let steps = driving_steps(
            &run_date_str,
            &next_day_str,
            &smelt_core::config::Granularity::Day,
        )?;
        run_windowed_keyed_maintenance(
            backend,
            &recipe.model_name,
            "main",
            &recipe.model_name,
            &steps,
            &classification,
            Some(&slice),
            compile_step,
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "composed route-3 window {i} (in-bound redelivery) unexpectedly refused: {e}"
            )
        })?;

        assert_composed_route3_equivalence(backend, recipe).await?;
        assert_composed_route3_per_slice(backend, recipe).await?;
    }
    Ok(())
}

/// `composed_keyed_pool_upholds_equivalence` (plan Phase A6 TDD list): the
/// standing proptest gate over the composed pool — deterministic seed,
/// small N. Each case draws one of the three [`ComposedRoute`]s, stages
/// it, confirms the real locality gate admits it with the expected slice
/// shape, then drives its equivalence check (route 1 through real
/// `execute_project`; routes 2/3 through the direct driver against a real
/// `DuckDbBackend`) — asserting equivalence, and a per-slice probe, after
/// **every** step.
#[test]
fn composed_keyed_pool_upholds_equivalence() {
    let n = composed_case_count();
    let mut runner = TestRunner::deterministic();
    let route_strat = arb_composed_route();
    let keyed_schedule_strat = arb_keyed_schedule();
    let route3_schedule_strat = arb_composed_route3_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut admitted_cases = 0;
    for i in 0..n {
        let route = route_strat.new_tree(&mut runner).unwrap().current();
        let recipe = ComposedKeyedRecipe::new(route);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = render::stage_composed(&recipe, &project_dir, &db_path).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} failed to stage: {e}")
        });

        let (plan, diags) = classify_composed_full(&project, &recipe).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} classify failed: {e}")
        });
        let plan = plan.unwrap_or_else(|| {
            panic!("case {i}: no plan derived for {recipe:?}: diagnostics={diags:#?}")
        });
        assert_composed_admitted_with_expected_route(&recipe, &plan).unwrap_or_else(|e| {
            panic!("case {i}: {e}");
        });
        admitted_cases += 1;

        match route {
            ComposedRoute::KeyEmbedded => {
                let schedule = keyed_schedule_strat
                    .new_tree(&mut runner)
                    .unwrap()
                    .current();
                rt.block_on(drive_composed_route1_and_assert(
                    &project, &recipe, &schedule,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: composed route-1 recipe {recipe:?} schedule {schedule:?} \
                         failed: {e}"
                    )
                });
            }
            ComposedRoute::KeyDetermined => {
                let schedule = keyed_schedule_strat
                    .new_tree(&mut runner)
                    .unwrap()
                    .current();
                let backend = rt.block_on(async {
                    DuckDbBackend::new(&project.db_path, "main")
                        .await
                        .expect("open backend")
                });
                rt.block_on(drive_composed_route2_and_assert(
                    &backend, &recipe, &schedule,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: composed route-2 recipe {recipe:?} schedule {schedule:?} \
                         failed: {e}"
                    )
                });
            }
            ComposedRoute::RecurrenceBounded => {
                let schedule = route3_schedule_strat
                    .new_tree(&mut runner)
                    .unwrap()
                    .current();
                let backend = rt.block_on(async {
                    DuckDbBackend::new(&project.db_path, "main")
                        .await
                        .expect("open backend")
                });
                rt.block_on(drive_composed_route3_and_assert(
                    &backend, &recipe, &schedule,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: composed route-3 recipe {recipe:?} schedule {schedule:?} \
                         failed: {e}"
                    )
                });
            }
        }
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic composed sample admitted zero cases — generator/derivation \
         regression"
    );
}

/// `composed_keyed_admission_rate_stays_above_floor` (plan Phase A6 TDD
/// list): generator health — every generated composed recipe is
/// deliberately constructed to admit exactly its own route (unlike the
/// append-only-partition pool's randomly-drawn constructs, this pool has
/// no randomised refusal branch), so the floor is high: a regression that
/// silently breaks one route's admission must fail this test rather than
/// hollow out the standing gate above unnoticed.
#[test]
fn composed_keyed_admission_rate_stays_above_floor() {
    const N: usize = 30;
    let mut runner = TestRunner::deterministic();
    let route_strat = arb_composed_route();

    let mut admitted = 0;
    for i in 0..N {
        let route = route_strat.new_tree(&mut runner).unwrap().current();
        let recipe = ComposedKeyedRecipe::new(route);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = render::stage_composed(&recipe, &project_dir, &db_path).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} failed to stage: {e}")
        });
        let (plan, _diags) = classify_composed_full(&project, &recipe).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} classify failed: {e}")
        });
        let admitted_here = plan
            .map(|p| assert_composed_admitted_with_expected_route(&recipe, &p).is_ok())
            .unwrap_or(false);
        if admitted_here {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / N as f64;
    assert!(
        rate >= 0.90,
        "composed-pool admission rate {rate:.2} over N={N} fell below the 90% floor \
         ({admitted}/{N} admitted) — a route-admission regression would silently hollow out the \
         standing gate"
    );
}
