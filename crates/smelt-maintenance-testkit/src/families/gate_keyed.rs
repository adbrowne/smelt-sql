//! The `grain: key` `KeyedRecipe` pool family — the target-parametrized twin
//! of `maintenance_conformance/gate.rs`'s keyed leg. Staging, row insertion,
//! snapshot reads, S-materialization, and the multiset comparison all route
//! through [`smelt_backend::Backend`].

use anyhow::Result;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_backend::Backend;

use crate::families::ConformanceBackend;
use crate::link_c_harness::{base_request, LinkCProject};
use crate::oracle::multiset_equal_via_backend_with_diff;
use crate::recipe::{
    arb_keyed_schedule, ConformanceTarget, KeyedCombiner, KeyedRecipe, KeyedSchedule,
};
use crate::render;
use crate::s_tracker::STracker;
use crate::schedule_gen::{read_source_snapshot_via_backend, GenRow};
use crate::verdict::{classify_keyed, Verdict};

/// Stage a [`KeyedRecipe`] into a fresh temp project dir targeting `target`
/// (`render::stage_keyed_for_target`'s drop-before-seed idempotency).
pub fn stage_keyed_recipe_for(
    recipe: &KeyedRecipe,
    tmp: &tempfile::TempDir,
    target: ConformanceTarget,
) -> Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("unused.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_keyed_for_target(recipe, &project_dir, &db_path, target)
}

/// Insert one row into a [`KeyedRecipe`]'s staged driving-source table via
/// `Backend::execute_sql`.
pub async fn insert_row_keyed_for(
    backend: &dyn Backend,
    schema: &str,
    recipe: &KeyedRecipe,
    row: &GenRow,
) -> Result<()> {
    backend
        .execute_sql(&format!(
            "INSERT INTO {schema}.sources_{name} VALUES (DATE '{d}', {id}, {val})",
            name = recipe.source.name,
            d = row.d.format("%Y-%m-%d"),
            id = row.id,
            val = row.val,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("insert keyed row: {e}"))?;
    Ok(())
}

/// The end-state equivalence assertion for a [`KeyedRecipe`] — resolves
/// `S_k`'s oracle relation through
/// [`ConformanceBackend::oracle_relation`]: a session-scoped temp VIEW
/// (`STracker::materialize_s_as_view`) for DuckDB/Spark, an inline derived
/// table for BigQuery
/// (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 7).
pub async fn assert_keyed_equivalence_for(
    b: &dyn ConformanceBackend,
    schema: &str,
    recipe: &KeyedRecipe,
    backend: &dyn Backend,
    tracker: &STracker,
    k: usize,
) -> Result<()> {
    let relation = b.oracle_relation(backend, tracker, k).await?;
    let maintained_sql = format!("SELECT * FROM {schema}.{}", recipe.model_name);
    let oracle_sql = render::render_keyed_oracle_body_over(recipe, &relation);
    let equal =
        multiset_equal_via_backend_with_diff(backend, &maintained_sql, &oracle_sql, |l, r| {
            b.multiset_diff_sql(l, r)
        })
        .await?;
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
/// window-forward posture) through the real `execute_project` pipeline
/// (`LinkCProject::run_with_target`), asserting end-state equivalence after
/// every window.
pub async fn drive_keyed_and_assert_for(
    b: &dyn ConformanceBackend,
    case: usize,
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    schedule: &KeyedSchedule,
) -> Result<()> {
    let target = b.target(case);
    let schema = b.schema(case);
    let backend = project.backend_for_target(target.clone()).await?;
    let mut tracker = STracker::new(&recipe.source);

    for (i, window) in schedule.0.iter().enumerate() {
        for row in &window.rows {
            b.before_step().await;
            insert_row_keyed_for(backend.as_ref(), &schema, recipe, row).await?;
        }

        let snapshot =
            read_source_snapshot_via_backend(backend.as_ref(), &schema, &recipe.source).await?;

        let mut request = base_request(b.engine_name());
        request.start = Some(window.start.format("%Y-%m-%d").to_string());
        request.end = Some(window.end.format("%Y-%m-%d").to_string());
        b.before_step().await;
        project
            .run_with_target(
                target.clone(),
                &format!("{}-keyed-run-{i}", b.engine_name()),
                request,
                &smelt_runtime::NoOpReporter,
            )
            .await?;

        let k = tracker.record_run(window.start, window.end, snapshot);
        assert_keyed_equivalence_for(b, &schema, recipe, backend.as_ref(), &tracker, k).await?;
    }
    Ok(())
}

/// `keyed_pool_upholds_end_state_equivalence` (the twin of
/// `maintenance_conformance::gate::keyed_pool_upholds_end_state_equivalence`)
/// — restricted to [`KeyedCombiner::Idempotent`] only. The DuckDB leg
/// samples both combiner families via `arb_keyed_combiner()`, but
/// [`KeyedCombiner::Additive`] grades `Grade::Additive`, whose
/// never-fold-twice reconciliation ledger is DuckDB-only today
/// (`maintenance_driver.rs`'s `Grade::Additive` arm fails loud with
/// `BackendError::unsupported` for any non-DuckDB dialect — MP12's ledger DDL
/// has no non-DuckDB dialect yet). `KeyedCombiner::Idempotent` grades
/// `Grade::Idempotent`, whose watermark-only ledger has no such dialect
/// restriction.
pub async fn run_keyed_pool_upholds_end_state_equivalence(
    b: &dyn ConformanceBackend,
    n: usize,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let schedule_strat = arb_keyed_schedule();

    let mut admitted_cases = 0;
    for i in 0..n {
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        let recipe = KeyedRecipe::new_window_forward(KeyedCombiner::Idempotent);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe_for(&recipe, &tmp, b.target(i))
            .unwrap_or_else(|e| panic!("case {i}: keyed recipe {recipe:?} failed to stage: {e}"));

        let verdict = classify_keyed(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: keyed recipe {recipe:?} classify failed: {e}"));
        anyhow::ensure!(
            matches!(verdict, Verdict::Admitted(_)),
            "case {i}: keyed recipe {recipe:?} admitted zero cells — generator/derivation \
             regression: {verdict:?}"
        );
        admitted_cases += 1;

        drive_keyed_and_assert_for(b, i, &project, &recipe, &schedule)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: keyed recipe {recipe:?} schedule {schedule:?} equivalence check \
                     failed: {e}"
                )
            });
    }

    anyhow::ensure!(
        admitted_cases > 0,
        "N={n} deterministic keyed sample admitted zero cases — generator/derivation regression"
    );
    Ok(())
}
