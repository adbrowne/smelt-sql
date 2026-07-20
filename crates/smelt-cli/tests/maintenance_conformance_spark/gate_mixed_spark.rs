//! Spark twin of `maintenance_conformance/gate.rs`'s fact+mutable-dimension
//! mixed pool (`docs/plans/20260720-prod-w9-spark-conformance-twin.md`
//! Phase 4): the same [`MutableEnrichedRecipe`] shape and settled-point
//! oracle, driven against a live Spark Connect/Delta backend.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_backend::Backend;
use smelt_maintenance_testkit::link_c_harness::{
    base_request, open_spark_conformance_backend, LinkCProject,
};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{
    ConformanceTarget, MutableEnrichedRecipe, SPARK_CONFORMANCE_SCHEMA,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{
    arb_mixed_schedule, check_profile as check_mixed_schedule_profile, GenRow, MixedSchedule,
    MixedStep,
};

use crate::gate_spark::spark_connect_url;

/// Default deterministic case count for
/// `mutable_pool_settles_to_full_refresh_on_spark` —
/// `SMELT_CONFORMANCE_SPARK_MIXED_CASES` env override, smaller than the
/// DuckDB leg's 6.
const MIXED_DEFAULT_CASES: usize = 2;

fn mixed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_MIXED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MIXED_DEFAULT_CASES)
}

/// Mirrors `maintenance_conformance/gate.rs::MIXED_DIM_SEED_MAX_ID` — the
/// upper bound on distinct fact/dimension ids a generated [`MixedSchedule`]
/// can ever produce, so [`stage_mixed_recipe_spark`] can pre-seed every
/// dimension row a schedule might reference.
const MIXED_DIM_SEED_MAX_ID: i64 = 12;

/// Stage a [`MutableEnrichedRecipe`] targeting Spark/Delta: writes the model,
/// both source YAMLs, and a Spark-targeted `smelt.yml`, then drops and
/// recreates both physical source tables (dropping any stale maintained
/// model table too) through a direct Spark/Delta connection, seeding the
/// dimension table with one row per id in `1..=MIXED_DIM_SEED_MAX_ID` — the
/// Spark counterpart of `maintenance_conformance/gate.rs::stage_mixed_recipe`.
/// Drop-before-seed idempotent, mirroring
/// `render::reset_and_create_spark_source_table`'s convention, since the
/// Delta warehouse persists across test invocations.
fn stage_mixed_recipe_spark(
    recipe: &MutableEnrichedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("unused.duckdb");
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
        render::render_smelt_yml_for(ConformanceTarget::SparkDelta, &db_path),
    )?;

    reset_and_seed_spark_mixed_tables(&db_path, recipe)?;

    LinkCProject::load(project_dir, db_path)
}

fn reset_and_seed_spark_mixed_tables(
    db_path: &std::path::Path,
    recipe: &MutableEnrichedRecipe,
) -> anyhow::Result<()> {
    let db_path = db_path.to_path_buf();
    let recipe = recipe.clone();
    // A fresh single-use runtime, not the shared isolated-thread helper
    // (`link_c_harness::block_on_isolated` is `pub(crate)` to the testkit
    // crate, unreachable from this test binary) — safe here because every
    // caller of this function ([`stage_mixed_recipe_spark`]) runs from a
    // plain synchronous `#[test]` body, never from inside an already-active
    // Tokio runtime.
    tokio::runtime::Runtime::new()
        .unwrap_or_else(|e| panic!("tokio runtime for Spark mixed-pool staging DDL: {e}"))
        .block_on(async move {
            let backend = open_spark_conformance_backend(&db_path).await?;
            let schema = SPARK_CONFORMANCE_SCHEMA;

            backend
                .execute_sql(&format!(
                    "DROP TABLE IF EXISTS {schema}.{}",
                    recipe.model_name
                ))
                .await
                .map_err(|e| anyhow::anyhow!("drop stale Spark mixed model table: {e}"))?;
            backend
                .execute_sql(&format!(
                    "DROP TABLE IF EXISTS {schema}.sources_{}",
                    recipe.fact.name
                ))
                .await
                .map_err(|e| anyhow::anyhow!("drop stale Spark mixed fact table: {e}"))?;
            backend
                .execute_sql(&format!(
                    "DROP TABLE IF EXISTS {schema}.sources_{}",
                    recipe.dimension.name
                ))
                .await
                .map_err(|e| anyhow::anyhow!("drop stale Spark mixed dimension table: {e}"))?;

            backend
                .execute_sql(&format!(
                "CREATE TABLE {schema}.sources_{fact} ({d} DATE, {id} INT, {val} INT) USING DELTA",
                fact = recipe.fact.name,
                d = recipe.fact.clock_column,
                id = recipe.fact.key_column,
                val = recipe.fact.payload_column,
            ))
                .await
                .map_err(|e| anyhow::anyhow!("create Spark mixed fact table: {e}"))?;
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {schema}.sources_{dim} ({dim_id} INT, {attr} INT) USING DELTA",
                    dim = recipe.dimension.name,
                    dim_id = recipe.dimension.key_column,
                    attr = recipe.dimension.payload_column,
                ))
                .await
                .map_err(|e| anyhow::anyhow!("create Spark mixed dimension table: {e}"))?;

            for id in 1..=MIXED_DIM_SEED_MAX_ID {
                backend
                    .execute_sql(&format!(
                        "INSERT INTO {schema}.sources_{} VALUES ({}, {})",
                        recipe.dimension.name,
                        id,
                        id * 100
                    ))
                    .await
                    .map_err(|e| anyhow::anyhow!("seed Spark mixed dimension row {id}: {e}"))?;
            }
            Ok::<(), anyhow::Error>(())
        })
}

/// Classify a staged [`MutableEnrichedRecipe`] through the real maintenance
/// derivation — copied from `maintenance_conformance/gate.rs::classify_mixed`
/// (target-agnostic, see `gate_keyed_spark.rs::classify_keyed_full`'s doc
/// comment for why this is a copy rather than a shared import).
fn classify_mixed(
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

/// Insert one fact row into `recipe`'s staged Spark/Delta fact source table.
async fn insert_fact_row_spark(
    backend: &dyn Backend,
    recipe: &MutableEnrichedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    backend
        .execute_sql(&format!(
            "INSERT INTO {schema}.sources_{name} VALUES (DATE '{d}', {id}, {val})",
            schema = SPARK_CONFORMANCE_SCHEMA,
            name = recipe.fact.name,
            d = row.d.format("%Y-%m-%d"),
            id = row.id,
            val = row.val,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("insert mixed fact row on Spark: {e}"))?;
    Ok(())
}

/// The window a `MutateDimension` step's `id` was produced in — copied from
/// `maintenance_conformance/gate.rs::fact_window_for_id` (pure function over
/// `MixedSchedule`, no backend dependency).
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

/// The settled-point oracle assertion on Spark (mirrors
/// `maintenance_conformance/gate.rs::assert_mixed_settled`): the fact side
/// reads the S-restricted temp view (`tracker`'s `S_k`, materialized via
/// [`STracker::materialize_s_as_view`] — Spark has no `CREATE TEMP TABLE`);
/// the dimension side always reads its CURRENT physical state.
async fn assert_mixed_settled_spark(
    recipe: &MutableEnrichedRecipe,
    backend: &dyn Backend,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    tracker.materialize_s_as_view(backend, k).await?;
    let maintained_sql = format!(
        "SELECT * FROM {SPARK_CONFORMANCE_SCHEMA}.{}",
        recipe.model_name
    );
    // `STracker::materialize_s_as_view` names its view `oracle_<source_name>`
    // — mirrored here rather than exposed as a public accessor, matching
    // `gate.rs::assert_mixed_settled`'s own convention. Built by hand here
    // rather than via `MutableEnrichedRecipe::oracle_body_over` — that
    // method hardcodes the dimension's physical table reference to
    // `main.sources_<name>` (the DuckDB-only schema), so it cannot be reused
    // for the Spark leg's `SPARK_CONFORMANCE_SCHEMA`-qualified table.
    let oracle_sql = recipe
        .model_body()
        .replace(
            &format!("smelt.sources.{}", recipe.fact.name),
            &format!("oracle_{}", recipe.fact.name),
        )
        .replace(
            &format!("smelt.sources.{}", recipe.dimension.name),
            &format!(
                "{SPARK_CONFORMANCE_SCHEMA}.sources_{}",
                recipe.dimension.name
            ),
        );
    let equal = multiset_equal_via_backend(backend, &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "Spark settled-point equivalence violated for {:?} at run {k}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` (a [`MutableEnrichedRecipe`])
/// through the real `execute_project` pipeline on Spark — mirrors
/// `maintenance_conformance/gate.rs::drive_mixed_and_assert`.
async fn drive_mixed_and_assert_spark(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
    schedule: &MixedSchedule,
) -> anyhow::Result<()> {
    let backend = project
        .backend_for_target(ConformanceTarget::SparkDelta)
        .await?;
    let mut tracker = STracker::new(&recipe.fact);
    let mut settled_assertions = 0usize;

    for (i, step) in schedule.0.iter().enumerate() {
        match step {
            MixedStep::RunWindow { start, end, rows } => {
                for row in rows {
                    insert_fact_row_spark(backend.as_ref(), recipe, row).await?;
                }

                let mut request = base_request("spark");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project
                    .run_with_target(
                        ConformanceTarget::SparkDelta,
                        &format!("spark-mixed-run-{i}"),
                        request,
                        &smelt_runtime::NoOpReporter,
                    )
                    .await?;

                let snapshot =
                    smelt_maintenance_testkit::schedule_gen::read_source_snapshot_via_backend(
                        backend.as_ref(),
                        SPARK_CONFORMANCE_SCHEMA,
                        &recipe.fact,
                    )
                    .await?;
                let k = tracker.record_run(*start, *end, snapshot);

                match tracker.oracle_mode() {
                    smelt_maintenance_testkit::oracle_modes::OracleMode::SRestricted => {
                        assert_mixed_settled_spark(recipe, backend.as_ref(), &tracker, k).await?;
                        settled_assertions += 1;
                    }
                    smelt_maintenance_testkit::oracle_modes::OracleMode::SettledPoint => {
                        // Expected staleness: some window's dimension
                        // mutation has not yet been caught up — non-fatal.
                    }
                }
            }
            MixedStep::MutateDimension { id, new_attr } => {
                backend
                    .execute_sql(&format!(
                        "UPDATE {schema}.sources_{name} SET {attr} = {new_attr} WHERE {key} = {id}",
                        schema = SPARK_CONFORMANCE_SCHEMA,
                        name = recipe.dimension.name,
                        attr = recipe.dimension.payload_column,
                        key = recipe.dimension.key_column,
                    ))
                    .await
                    .map_err(|e| anyhow::anyhow!("mutate Spark mixed dimension row: {e}"))?;

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
        "schedule {schedule:?} never reached a settled point on Spark — no assertion ran"
    );
    Ok(())
}

/// `mutable_pool_settles_to_full_refresh_on_spark` (plan Phase 4 TDD list):
/// the Spark twin of
/// `maintenance_conformance::gate::mutable_pool_settles_to_full_refresh` —
/// fact+mutable-dimension recipes hold full equivalence at every settled
/// point on Spark/Delta too; expected-staleness in between is recorded,
/// never fatal.
#[test]
fn mutable_pool_settles_to_full_refresh_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping mutable_pool_settles_to_full_refresh_on_spark"
        );
        return;
    };

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
        let project = stage_mixed_recipe_spark(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: failed to stage mixed recipe on Spark: {e}"));

        let plan = classify_mixed(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: classify_mixed failed on Spark: {e}"));
        assert!(
            !plan.cells.is_empty(),
            "case {i}: mixed recipe {recipe:?} admitted zero cells on Spark — \
             generator/derivation regression (expected at least the UpstreamMutation \
             ColumnScopedMerge cell)"
        );

        rt.block_on(drive_mixed_and_assert_spark(&project, &recipe, &schedule))
            .unwrap_or_else(|e| {
                panic!("case {i}: mixed schedule {schedule:?} Spark equivalence check failed: {e}")
            });
    }
}
