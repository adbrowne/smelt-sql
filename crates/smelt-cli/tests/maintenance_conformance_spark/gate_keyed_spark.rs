//! Spark twin of `maintenance_conformance/gate.rs`'s `grain: key` pool
//! (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 4): the
//! same `KeyedRecipe` pool and window-forward schedule driver, driven
//! against a live Spark Connect/Delta backend instead of DuckDB. Staging,
//! row insertion, snapshot reads, S-materialization, and the multiset
//! comparison all route through `smelt_backend::Backend`.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{
    arb_keyed_schedule, ConformanceTarget, KeyedCombiner, KeyedRecipe, KeyedSchedule,
    SPARK_CONFORMANCE_SCHEMA,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{read_source_snapshot_via_backend, GenRow};
use smelt_maintenance_testkit::verdict::{classify_keyed, Verdict};

use crate::gate_spark::spark_connect_url;

/// Default deterministic case count for
/// `keyed_pool_upholds_end_state_equivalence_on_spark` —
/// `SMELT_CONFORMANCE_SPARK_KEYED_CASES` env override. Smaller than the
/// DuckDB leg's default (6): each case drives several `execute_project`
/// windows over a live Spark Connect server.
const DEFAULT_KEYED_CASES: usize = 3;

fn keyed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_KEYED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_KEYED_CASES)
}

/// Stage a [`KeyedRecipe`] targeting Spark/Delta into a fresh temp project
/// dir (`render::stage_keyed_for_target`'s Spark arm) — drop-before-seed
/// idempotency, mirroring `gate_spark::stage_recipe_spark`.
pub(crate) fn stage_keyed_recipe_spark(
    recipe: &KeyedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("unused.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_keyed_for_target(
        recipe,
        &project_dir,
        &db_path,
        ConformanceTarget::SparkDelta,
    )
}

/// Insert one row into a [`KeyedRecipe`]'s staged Spark/Delta driving-source
/// table via `Backend::execute_sql` — mirrors `gate.rs::insert_row_keyed`.
async fn insert_row_keyed_spark(
    backend: &dyn smelt_backend::Backend,
    recipe: &KeyedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    backend
        .execute_sql(&format!(
            "INSERT INTO {schema}.sources_{name} VALUES (DATE '{d}', {id}, {val})",
            schema = SPARK_CONFORMANCE_SCHEMA,
            name = recipe.source.name,
            d = row.d.format("%Y-%m-%d"),
            id = row.id,
            val = row.val,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("insert keyed row on Spark: {e}"))?;
    Ok(())
}

/// The end-state equivalence assertion for a [`KeyedRecipe`] on Spark —
/// mirrors `gate.rs::assert_keyed_equivalence`, materializing `S_k` as a temp
/// VIEW (`STracker::materialize_s_as_view` — Spark has no `CREATE TEMP
/// TABLE`) rather than a temp table.
async fn assert_keyed_equivalence_spark(
    recipe: &KeyedRecipe,
    backend: &dyn smelt_backend::Backend,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    tracker.materialize_s_as_view(backend, k).await?;
    let maintained_sql = format!(
        "SELECT * FROM {SPARK_CONFORMANCE_SCHEMA}.{}",
        recipe.model_name
    );
    // `tracker.materialize_s_as_view` creates an unqualified (session-scoped)
    // temp view named `oracle_<source_name>` — the same name
    // `render_keyed_oracle_body_over` expects, mirroring
    // `gate.rs::assert_keyed_equivalence`'s DuckDB call exactly.
    let oracle_sql =
        render::render_keyed_oracle_body_over(recipe, &format!("oracle_{}", recipe.source.name));
    let equal = multiset_equal_via_backend(backend, &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "Spark keyed end-state equivalence violated for model {:?} at run {k}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` (a [`KeyedRecipe`] under the
/// window-forward posture) on Spark/Delta through the real `execute_project`
/// pipeline (`LinkCProject::run_with_target`), asserting end-state
/// equivalence after every window — mirrors `gate.rs::drive_keyed_and_assert`.
pub(crate) async fn drive_keyed_and_assert_spark(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    schedule: &KeyedSchedule,
) -> anyhow::Result<()> {
    let backend = project
        .backend_for_target(ConformanceTarget::SparkDelta)
        .await?;
    let mut tracker = STracker::new(&recipe.source);

    for (i, window) in schedule.0.iter().enumerate() {
        for row in &window.rows {
            insert_row_keyed_spark(backend.as_ref(), recipe, row).await?;
        }

        let snapshot = read_source_snapshot_via_backend(
            backend.as_ref(),
            SPARK_CONFORMANCE_SCHEMA,
            &recipe.source,
        )
        .await?;

        let mut request = base_request("spark");
        request.start = Some(window.start.format("%Y-%m-%d").to_string());
        request.end = Some(window.end.format("%Y-%m-%d").to_string());
        project
            .run_with_target(
                ConformanceTarget::SparkDelta,
                &format!("spark-keyed-run-{i}"),
                request,
                &smelt_runtime::NoOpReporter,
            )
            .await?;

        let k = tracker.record_run(window.start, window.end, snapshot);
        assert_keyed_equivalence_spark(recipe, backend.as_ref(), &tracker, k).await?;
    }
    Ok(())
}

/// `keyed_pool_upholds_end_state_equivalence_on_spark` (plan Phase 4 TDD
/// list): the Spark twin of
/// `maintenance_conformance::gate::keyed_pool_upholds_end_state_equivalence`
/// — keyed recipes equal the oracle's end state at settled points on
/// Spark/Delta too.
///
/// Restricted to [`KeyedCombiner::Idempotent`] only — the DuckDB leg samples
/// both combiner families via `arb_keyed_combiner()`, but
/// [`KeyedCombiner::Additive`] grades `Grade::Additive`, whose
/// never-fold-twice reconciliation ledger is DuckDB-only today
/// (`maintenance_driver.rs`'s `Grade::Additive` arm fails loud with
/// `BackendError::unsupported` for any non-DuckDB dialect — MP12's ledger DDL
/// has no Spark dialect yet). This is a genuine, pre-existing, already
/// fail-loud backend gap (not a W9 regression) — see this plan's "Deferred
/// during implementation" entry. `KeyedCombiner::Idempotent` grades
/// `Grade::Idempotent`, whose watermark-only ledger has no such dialect
/// restriction and is the leg this test actually exercises.
#[test]
fn keyed_pool_upholds_end_state_equivalence_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             keyed_pool_upholds_end_state_equivalence_on_spark"
        );
        return;
    };

    let n = keyed_case_count();
    let mut runner = TestRunner::deterministic();
    let schedule_strat = arb_keyed_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut admitted_cases = 0;
    for i in 0..n {
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        let recipe = KeyedRecipe::new_window_forward(KeyedCombiner::Idempotent);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe_spark(&recipe, &tmp).unwrap_or_else(|e| {
            panic!("case {i}: keyed recipe {recipe:?} failed to stage on Spark: {e}")
        });

        let verdict = classify_keyed(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: keyed recipe {recipe:?} classify failed: {e}"));
        assert!(
            matches!(verdict, Verdict::Admitted(_)),
            "case {i}: keyed recipe {recipe:?} admitted zero cells on Spark — \
             generator/derivation regression: {verdict:?}"
        );
        admitted_cases += 1;

        rt.block_on(drive_keyed_and_assert_spark(&project, &recipe, &schedule))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: keyed recipe {recipe:?} schedule {schedule:?} Spark equivalence \
                     check failed: {e}"
                )
            });
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic keyed sample admitted zero cases on Spark — generator/derivation \
         regression"
    );
}
