//! The fact+mutable-dimension mixed pool family (`MutableEnrichedRecipe`) —
//! the target-parametrized twin of `maintenance_conformance/gate.rs`'s
//! mixed leg.

use anyhow::Result;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_backend::Backend;

use crate::families::ConformanceBackend;
use crate::link_c_harness::{base_request, LinkCProject};
use crate::oracle::multiset_equal_via_backend_with_diff;
use crate::recipe::MutableEnrichedRecipe;
use crate::render;
use crate::s_tracker::STracker;
use crate::schedule_gen::{
    arb_mixed_schedule, check_profile as check_mixed_schedule_profile, GenRow, MixedSchedule,
    MixedStep,
};

/// Mirrors `maintenance_conformance/gate.rs::MIXED_DIM_SEED_MAX_ID` — the
/// upper bound on distinct fact/dimension ids a generated [`MixedSchedule`]
/// can ever produce, so [`stage_mixed_recipe_for`] can pre-seed every
/// dimension row a schedule might reference.
const MIXED_DIM_SEED_MAX_ID: i64 = 12;

/// Stage a [`MutableEnrichedRecipe`] targeting `target`: writes the model,
/// both source YAMLs, and a `target`-scoped `smelt.yml`, then drops and
/// recreates both physical source tables (dropping any stale maintained
/// model table too) through a direct backend connection opened via
/// [`ConformanceBackend::open_backend`], seeding the dimension table with
/// one row per id in `1..=MIXED_DIM_SEED_MAX_ID`. Drop-before-seed
/// idempotent, since a persistent warehouse (Spark) may carry a stale table
/// from a prior run; a fresh-per-case dataset (BigQuery) needs no drop.
pub async fn stage_mixed_recipe_for(
    b: &dyn ConformanceBackend,
    case: usize,
    recipe: &MutableEnrichedRecipe,
    tmp: &tempfile::TempDir,
) -> Result<LinkCProject> {
    let target = b.target(case);
    let schema = b.schema(case);
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
        render::render_smelt_yml_for(target.clone(), &db_path),
    )?;

    reset_and_seed_mixed_tables(b, case, &db_path, &schema, recipe).await?;

    LinkCProject::load(project_dir, db_path)
}

async fn reset_and_seed_mixed_tables(
    b: &dyn ConformanceBackend,
    case: usize,
    db_path: &std::path::Path,
    schema: &str,
    recipe: &MutableEnrichedRecipe,
) -> Result<()> {
    let backend = b.open_backend(case, db_path).await?;
    let storage_clause = b.storage_clause();

    backend
        .execute_sql(&format!(
            "DROP TABLE IF EXISTS {schema}.{}",
            recipe.model_name
        ))
        .await
        .map_err(|e| anyhow::anyhow!("drop stale mixed model table: {e}"))?;
    backend
        .execute_sql(&format!(
            "DROP TABLE IF EXISTS {schema}.sources_{}",
            recipe.fact.name
        ))
        .await
        .map_err(|e| anyhow::anyhow!("drop stale mixed fact table: {e}"))?;
    backend
        .execute_sql(&format!(
            "DROP TABLE IF EXISTS {schema}.sources_{}",
            recipe.dimension.name
        ))
        .await
        .map_err(|e| anyhow::anyhow!("drop stale mixed dimension table: {e}"))?;

    backend
        .execute_sql(&format!(
            "CREATE TABLE {schema}.sources_{fact} ({d} DATE, {id} INT, {val} INT){storage_clause}",
            fact = recipe.fact.name,
            d = recipe.fact.clock_column,
            id = recipe.fact.key_column,
            val = recipe.fact.payload_column,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("create mixed fact table: {e}"))?;
    backend
        .execute_sql(&format!(
            "CREATE TABLE {schema}.sources_{dim} ({dim_id} INT, {attr} INT){storage_clause}",
            dim = recipe.dimension.name,
            dim_id = recipe.dimension.key_column,
            attr = recipe.dimension.payload_column,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("create mixed dimension table: {e}"))?;

    for id in 1..=MIXED_DIM_SEED_MAX_ID {
        backend
            .execute_sql(&format!(
                "INSERT INTO {schema}.sources_{} VALUES ({}, {})",
                recipe.dimension.name,
                id,
                id * 100
            ))
            .await
            .map_err(|e| anyhow::anyhow!("seed mixed dimension row {id}: {e}"))?;
    }
    Ok(())
}

/// Classify a staged [`MutableEnrichedRecipe`] through the real maintenance
/// derivation — target-agnostic: reads the staged project's own files off
/// disk, never touches a backend.
pub fn classify_mixed(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
) -> Result<smelt_logical::maintenance::MaintenancePlan> {
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

/// Insert one fact row into `recipe`'s staged fact source table.
pub async fn insert_fact_row_for(
    backend: &dyn Backend,
    schema: &str,
    recipe: &MutableEnrichedRecipe,
    row: &GenRow,
) -> Result<()> {
    backend
        .execute_sql(&format!(
            "INSERT INTO {schema}.sources_{name} VALUES (DATE '{d}', {id}, {val})",
            name = recipe.fact.name,
            d = row.d.format("%Y-%m-%d"),
            id = row.id,
            val = row.val,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("insert mixed fact row: {e}"))?;
    Ok(())
}

/// The window a `MutateDimension` step's `id` was produced in — pure
/// function over `MixedSchedule`, no backend dependency.
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

/// [`ConformanceBackend::oracle_relation`]'s bare-identifier default form
/// (`oracle_<name>`) is valid whether or not a trailing alias immediately
/// follows it in the caller's `FROM` position — `oracle_x f` parses as
/// "identifier, then alias `f`" either way. Its BigQuery derived-table form
/// (`(<select>) AS oracle_<name>`) is NOT: [`MutableEnrichedRecipe::model_body`]'s
/// fact-join template already supplies its own trailing alias (`f`)
/// immediately after the substitution point, so splicing in a SECOND `AS
/// oracle_<name>` alias would double-alias (a syntax error on every
/// dialect). This strips a derived table's `) AS oracle_<name>` suffix back
/// down to a bare `)`, leaving the bare-identifier form untouched — the
/// one call site the plan's Phase 7 "confirm this holds for gate_mixed"
/// instruction flagged as needing adaptation rather than a straight
/// substitution.
fn bare_relation_for_alias(relation: &str, oracle_table_name: &str) -> String {
    let aliased_suffix = format!(") AS {oracle_table_name}");
    match relation.strip_suffix(&aliased_suffix) {
        Some(prefix) => format!("{prefix})"),
        None => relation.to_string(),
    }
}

/// The settled-point oracle assertion: the fact side reads the S-restricted
/// oracle relation (`tracker`'s `S_k`, resolved through
/// [`ConformanceBackend::oracle_relation`] — a temp view for DuckDB/Spark,
/// an inline derived table for BigQuery,
/// `docs/plans/20260817-bigquery-generative-conformance.md` Phase 7); the
/// dimension side always reads its CURRENT physical state.
async fn assert_mixed_settled_for(
    b: &dyn ConformanceBackend,
    schema: &str,
    recipe: &MutableEnrichedRecipe,
    backend: &dyn Backend,
    tracker: &STracker,
    k: usize,
) -> Result<()> {
    let relation = b.oracle_relation(backend, tracker, k).await?;
    let fact_relation = bare_relation_for_alias(&relation, &tracker.oracle_table_name());
    let maintained_sql = format!("SELECT * FROM {schema}.{}", recipe.model_name);
    // Built by hand here rather than via `MutableEnrichedRecipe::
    // oracle_body_over` — that method hardcodes the dimension's physical
    // table reference to `main.sources_<name>` (the DuckDB-only schema), so
    // it cannot be reused for a `schema`-qualified table.
    let oracle_sql = recipe
        .model_body()
        .replace(
            &format!("smelt.sources.{}", recipe.fact.name),
            &fact_relation,
        )
        .replace(
            &format!("smelt.sources.{}", recipe.dimension.name),
            &format!("{schema}.sources_{}", recipe.dimension.name),
        );
    let equal =
        multiset_equal_via_backend_with_diff(backend, &maintained_sql, &oracle_sql, |l, r| {
            b.multiset_diff_sql(l, r)
        })
        .await?;
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
/// through the real `execute_project` pipeline.
pub async fn drive_mixed_and_assert_for(
    b: &dyn ConformanceBackend,
    case: usize,
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
    schedule: &MixedSchedule,
) -> Result<()> {
    let target = b.target(case);
    let schema = b.schema(case);
    let backend = project.backend_for_target(target.clone()).await?;
    let mut tracker = STracker::new(&recipe.fact);
    let mut settled_assertions = 0usize;

    for (i, step) in schedule.0.iter().enumerate() {
        match step {
            MixedStep::RunWindow { start, end, rows } => {
                for row in rows {
                    b.before_step().await;
                    insert_fact_row_for(backend.as_ref(), &schema, recipe, row).await?;
                }

                let mut request = base_request(b.engine_name());
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                b.before_step().await;
                project
                    .run_with_target(
                        target.clone(),
                        &format!("{}-mixed-run-{i}", b.engine_name()),
                        request,
                        &smelt_runtime::NoOpReporter,
                    )
                    .await?;

                let snapshot = crate::schedule_gen::read_source_snapshot_via_backend(
                    backend.as_ref(),
                    &schema,
                    &recipe.fact,
                )
                .await?;
                let k = tracker.record_run(*start, *end, snapshot);

                match tracker.oracle_mode() {
                    crate::oracle_modes::OracleMode::SRestricted => {
                        assert_mixed_settled_for(b, &schema, recipe, backend.as_ref(), &tracker, k)
                            .await?;
                        settled_assertions += 1;
                    }
                    crate::oracle_modes::OracleMode::SettledPoint => {
                        // Expected staleness: some window's dimension
                        // mutation has not yet been caught up — non-fatal.
                    }
                }
            }
            MixedStep::MutateDimension { id, new_attr } => {
                b.before_step().await;
                backend
                    .execute_sql(&format!(
                        "UPDATE {schema}.sources_{name} SET {attr} = {new_attr} WHERE {key} = {id}",
                        name = recipe.dimension.name,
                        attr = recipe.dimension.payload_column,
                        key = recipe.dimension.key_column,
                    ))
                    .await
                    .map_err(|e| anyhow::anyhow!("mutate mixed dimension row: {e}"))?;

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

/// `mutable_pool_settles_to_full_refresh` — the twin of
/// `maintenance_conformance::gate::mutable_pool_settles_to_full_refresh`.
pub async fn run_mutable_pool_settles_to_full_refresh(
    b: &dyn ConformanceBackend,
    n: usize,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let recipe = MutableEnrichedRecipe::new();
    let schedule_strat = arb_mixed_schedule();

    for i in 0..n {
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        check_mixed_schedule_profile(&schedule).unwrap_or_else(|e| {
            panic!("case {i}: generated mixed schedule {schedule:?} failed its own self-check: {e}")
        });

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_mixed_recipe_for(b, i, &recipe, &tmp)
            .await
            .unwrap_or_else(|e| panic!("case {i}: failed to stage mixed recipe: {e}"));

        let plan = classify_mixed(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: classify_mixed failed: {e}"));
        anyhow::ensure!(
            !plan.cells.is_empty(),
            "case {i}: mixed recipe {recipe:?} admitted zero cells — generator/derivation \
             regression (expected at least the UpstreamMutation ColumnScopedMerge cell)"
        );

        drive_mixed_and_assert_for(b, i, &project, &recipe, &schedule)
            .await
            .unwrap_or_else(|e| {
                panic!("case {i}: mixed schedule {schedule:?} equivalence check failed: {e}")
            });
    }
    Ok(())
}
