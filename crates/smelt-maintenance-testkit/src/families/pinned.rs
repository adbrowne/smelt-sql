//! The pinned catalogue-coverage + hazard-schedule family — the
//! target-parametrized twin of `maintenance_conformance/pinned.rs`:
//! reproduces the same construct × posture catalogue coverage and hazard
//! schedules, reusing the already-generic staging/drive helpers from
//! [`crate::families::gate`], [`crate::families::gate_keyed`], and
//! [`crate::families::gate_mixed`] rather than duplicating rendering logic.
//!
//! `hazard::keyed_merge_reprocessed_window` is the ONE hazard case NOT
//! ported: it specifically exercises `KeyedCombiner::Additive`'s
//! never-fold-twice REFUSAL, which requires the reconciliation ledger that
//! is DuckDB-only today (no non-DuckDB dialect for MP12's ledger DDL yet) —
//! on a non-DuckDB backend the very first fold in that hazard's own schedule
//! would fail loud with the ledger's own `BackendError::unsupported` rather
//! than the hazard's own `KeyedReprocessedWindow` refusal, so the case does
//! not reproduce (there is no non-DuckDB equivalent to pin).

use anyhow::Result;
use chrono::NaiveDate;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use crate::families::gate::{drive_and_assert_for, stage_recipe_for};
use crate::families::gate_keyed::stage_keyed_recipe_for;
use crate::families::gate_mixed::{classify_mixed, insert_fact_row_for, stage_mixed_recipe_for};
use crate::families::ConformanceBackend;
use crate::oracle::multiset_equal_via_backend_with_diff;
use crate::recipe::{
    arb_recipe, BodyConstruct, KeyedCombiner, KeyedRecipe, ModelRecipe, MutableEnrichedRecipe,
    RecipePool,
};
use crate::schedule_gen::{ConformanceSchedule, ConformanceStep, GenRow};
use crate::verdict::{classify, classify_keyed, Verdict};

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("valid base date")
        .checked_add_signed(chrono::Duration::days(offset))
        .expect("valid offset date")
}

/// Pure generator draw, no backend dependency, so identical regardless of
/// target.
fn pinned_body_recipe(pred: impl Fn(&BodyConstruct) -> bool) -> ModelRecipe {
    let mut runner = TestRunner::deterministic();
    let strat = arb_recipe(RecipePool::partition_append_only());
    for _ in 0..500 {
        let recipe = strat.new_tree(&mut runner).unwrap().current();
        if pred(&recipe.construct) {
            return recipe;
        }
    }
    panic!("deterministic sample never produced a matching recipe in 500 draws");
}

/// Pure data, no backend dependency.
fn minimal_schedule() -> ConformanceSchedule {
    ConformanceSchedule(vec![ConformanceStep::RunWindow {
        start: day(0),
        end: day(1),
        rows: vec![
            GenRow {
                d: day(0),
                id: 1,
                val: 10,
            },
            GenRow {
                d: day(0),
                id: 2,
                val: -5,
            },
        ],
    }])
}

async fn assert_pinned_body_recipe_is_green(b: &dyn ConformanceBackend, recipe: &ModelRecipe) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_for(recipe, &tmp, b.target(0))
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} failed to stage: {e}"));
    let verdict = classify(&project, recipe)
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} classify failed: {e}"));
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "pinned recipe {recipe:?} did not admit: {verdict:?}"
    );

    drive_and_assert_for(b, 0, &project, recipe, &minimal_schedule())
        .await
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} equivalence check failed: {e}"));
}

/// `pinned_recipes_reproduce_catalogue_coverage` — the twin of
/// `maintenance_conformance::pinned::pinned_recipes_reproduce_catalogue_coverage`.
pub async fn run_pinned_recipes_reproduce_catalogue_coverage(
    b: &dyn ConformanceBackend,
) -> Result<()> {
    assert_pinned_body_recipe_is_green(
        b,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::PassThrough)),
    )
    .await;
    assert_pinned_body_recipe_is_green(
        b,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::Filter { .. })),
    )
    .await;
    assert_pinned_body_recipe_is_green(
        b,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::AdditiveAgg)),
    )
    .await;
    assert_pinned_body_recipe_is_green(
        b,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::IdempotentAgg)),
    )
    .await;
    assert_pinned_body_recipe_is_green(
        b,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::DecomposedAgg)),
    )
    .await;
    assert_pinned_body_recipe_is_green(
        b,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::HolisticAgg)),
    )
    .await;

    let mixed = MutableEnrichedRecipe::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_mixed_recipe_for(b, 0, &mixed, &tmp)
        .await
        .unwrap_or_else(|e| panic!("stage mixed recipe: {e}"));
    let plan =
        classify_mixed(&project, &mixed).unwrap_or_else(|e| panic!("classify mixed recipe: {e}"));
    anyhow::ensure!(
        !plan.cells.is_empty(),
        "mutable-dimension-enrichment recipe admitted zero cells: {plan:#?}"
    );

    // Both `KeyedCombiner` families classify cleanly — admission is pure
    // classification, unaffected by the `Grade::Additive` ledger gap that
    // excludes execution of the Additive family elsewhere.
    for combiner in [KeyedCombiner::Additive, KeyedCombiner::Idempotent] {
        let recipe = KeyedRecipe::new_window_forward(combiner);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe_for(&recipe, &tmp, b.target(0))
            .unwrap_or_else(|e| panic!("stage keyed recipe {combiner:?}: {e}"));
        let verdict = classify_keyed(&project, &recipe)
            .unwrap_or_else(|e| panic!("classify keyed recipe {combiner:?}: {e}"));
        anyhow::ensure!(
            matches!(verdict, Verdict::Admitted(_)),
            "keyed recipe {combiner:?} did not admit: {verdict:?}"
        );
    }
    Ok(())
}

/// The retired hazard cases (see this module's doc comment for
/// `keyed_merge_reprocessed_window`'s exclusion).
mod hazard {
    use super::*;

    pub async fn additive_agg_append_only_control(b: &dyn ConformanceBackend) {
        assert_pinned_body_recipe_is_green(
            b,
            &pinned_body_recipe(|c| matches!(c, BodyConstruct::AdditiveAgg)),
        )
        .await;
    }

    pub async fn additive_agg_redelivery(b: &dyn ConformanceBackend) {
        let recipe = pinned_body_recipe(|c| matches!(c, BodyConstruct::AdditiveAgg));
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe_for(&recipe, &tmp, b.target(0)).expect("stage recipe");
        let verdict = classify(&project, &recipe).expect("classify");
        assert!(
            matches!(verdict, Verdict::Admitted(_)),
            "expected admission"
        );

        let schedule = ConformanceSchedule(vec![
            ConformanceStep::RunWindow {
                start: day(0),
                end: day(1),
                rows: vec![GenRow {
                    d: day(0),
                    id: 1,
                    val: 10,
                }],
            },
            ConformanceStep::RerunWindow {
                start: day(0),
                end: day(1),
            },
        ]);
        drive_and_assert_for(b, 0, &project, &recipe, &schedule)
            .await
            .unwrap_or_else(|e| panic!("redelivery schedule equivalence check failed: {e}"));
    }

    pub async fn idempotent_agg_append_only_control(b: &dyn ConformanceBackend) {
        assert_pinned_body_recipe_is_green(
            b,
            &pinned_body_recipe(|c| matches!(c, BodyConstruct::IdempotentAgg)),
        )
        .await;
    }

    pub async fn join_enrichment_mutable_dimension(b: &dyn ConformanceBackend) {
        let recipe = MutableEnrichedRecipe::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_mixed_recipe_for(b, 0, &recipe, &tmp)
            .await
            .expect("stage mixed recipe");
        let plan = classify_mixed(&project, &recipe).expect("classify mixed recipe");
        assert!(!plan.cells.is_empty(), "expected admission: {plan:#?}");

        let target = b.target(0);
        let schema = b.schema(0);
        let backend = project
            .backend_for_target(target.clone())
            .await
            .expect("open backend");

        insert_fact_row_for(
            backend.as_ref(),
            &schema,
            &recipe,
            &GenRow {
                d: day(0),
                id: 1,
                val: 10,
            },
        )
        .await
        .expect("insert fact row");
        let mut request = crate::link_c_harness::base_request(b.engine_name());
        request.start = Some("2024-01-01".to_string());
        request.end = Some("2024-01-02".to_string());
        project
            .run_with_target(
                target.clone(),
                &format!("pinned-g05-{}-run-1", b.engine_name()),
                request,
                &smelt_runtime::NoOpReporter,
            )
            .await
            .expect("first run");

        // Mutate the dimension in place — the exact hazard `G-05` named.
        backend
            .execute_sql(&format!(
                "UPDATE {schema}.sources_{name} SET {attr} = 999 WHERE {key} = 1",
                name = recipe.dimension.name,
                attr = recipe.dimension.payload_column,
                key = recipe.dimension.key_column,
            ))
            .await
            .expect("mutate dimension");

        // Re-run the SAME window: a catch-up run must reflect the CURRENT
        // dimension contents, not the stale one.
        let mut request = crate::link_c_harness::base_request(b.engine_name());
        request.start = Some("2024-01-01".to_string());
        request.end = Some("2024-01-02".to_string());
        project
            .run_with_target(
                target.clone(),
                &format!("pinned-g05-{}-run-2", b.engine_name()),
                request,
                &smelt_runtime::NoOpReporter,
            )
            .await
            .expect("catch-up run after dimension mutation");

        let maintained_sql = format!("SELECT * FROM {schema}.{}", recipe.model_name);
        let oracle_sql = recipe
            .model_body()
            .replace(
                &format!("smelt.sources.{}", recipe.dimension.name),
                &format!("{schema}.sources_{}", recipe.dimension.name),
            )
            .replace(
                &format!("smelt.sources.{}", recipe.fact.name),
                &format!("{schema}.sources_{}", recipe.fact.name),
            );
        assert!(
            multiset_equal_via_backend_with_diff(
                backend.as_ref(),
                &maintained_sql,
                &oracle_sql,
                |l, r| b.multiset_diff_sql(l, r)
            )
            .await
            .expect("catch-up multiset comparison"),
            "catch-up run after in-place dimension mutation diverged from a full-refresh \
             oracle over the CURRENT dimension contents"
        );
    }

    pub async fn holistic_agg_append_only_control(b: &dyn ConformanceBackend) {
        assert_pinned_body_recipe_is_green(
            b,
            &pinned_body_recipe(|c| matches!(c, BodyConstruct::HolisticAgg)),
        )
        .await;
    }

    /// `g_10_composite_key_join_fan_out`: a real fact/dim project where
    /// `dims` is unique only on the PAIR `(user_id, dt)`. Self-contained
    /// (mirrors `pinned.rs`'s own self-staged project).
    pub async fn composite_key_join_fan_out(b: &dyn ConformanceBackend) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().to_path_buf();
        let db_path = project_dir.join("unused.duckdb");
        std::fs::create_dir_all(project_dir.join("models/sources")).expect("mkdir");

        std::fs::write(
            project_dir.join("models/facts_enriched.sql"),
            r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
maintenance:
  scan_bounds:
    per_source:
      dims:
        allow_full_scan: true
---
SELECT f.d, f.user_id, f.dt, f.val, dm.payload
FROM smelt.sources.facts f
JOIN smelt.sources.dims dm ON f.user_id = dm.user_id AND f.dt = dm.dt
"#,
        )
        .expect("write model");
        std::fs::write(
            project_dir.join("models/sources/facts.yml"),
            "description: pinned g-10 fact source.\ncolumns:\n  - name: d\n    type: DATE\n  - name: user_id\n    type: BIGINT\n  - name: dt\n    type: BIGINT\n  - name: val\n    type: DOUBLE\ntimeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n",
        )
        .expect("write facts source");
        std::fs::write(
            project_dir.join("models/sources/dims.yml"),
            "description: pinned g-10 composite-key dimension source.\ncolumns:\n  - name: user_id\n    type: BIGINT\n  - name: dt\n    type: BIGINT\n  - name: payload\n    type: BIGINT\n",
        )
        .expect("write dims source");
        let target = b.target(0);
        let schema = b.schema(0);
        std::fs::write(
            project_dir.join("smelt.yml"),
            crate::render::render_smelt_yml_for(target.clone(), &db_path),
        )
        .expect("write smelt.yml");

        let backend = b
            .open_backend(0, &db_path)
            .await
            .expect("open backend for g-10 staging");
        let storage_clause = b.storage_clause();
        for table in ["facts_enriched", "sources_facts", "sources_dims"] {
            backend
                .execute_sql(&format!("DROP TABLE IF EXISTS {schema}.{table}"))
                .await
                .expect("drop stale g-10 table");
        }
        backend
            .execute_sql(&format!(
                "CREATE TABLE {schema}.sources_facts (d DATE, user_id INT, dt INT, val \
                 DOUBLE){storage_clause}"
            ))
            .await
            .expect("create g-10 facts table");
        backend
            .execute_sql(&format!(
                "INSERT INTO {schema}.sources_facts VALUES (DATE '2024-01-01', 1, 100, 10.0), \
                 (DATE '2024-01-01', 2, 200, 20.0)"
            ))
            .await
            .expect("seed g-10 facts");
        backend
            .execute_sql(&format!(
                "CREATE TABLE {schema}.sources_dims (user_id INT, dt INT, payload \
                 INT){storage_clause}"
            ))
            .await
            .expect("create g-10 dims table");
        backend
            .execute_sql(&format!(
                "INSERT INTO {schema}.sources_dims VALUES (1, 100, 111), (1, 200, 222), (2, \
                 100, 333), (2, 200, 444)"
            ))
            .await
            .expect("seed g-10 dims");

        let project = crate::link_c_harness::LinkCProject::load(project_dir, db_path)
            .expect("load g-10 project");
        let mut request = crate::link_c_harness::base_request(b.engine_name());
        request.start = Some("2024-01-01".to_string());
        request.end = Some("2024-01-02".to_string());
        project
            .run_with_target(
                target,
                &format!("pinned-g10-{}", b.engine_name()),
                request,
                &smelt_runtime::NoOpReporter,
            )
            .await
            .expect("g-10 run must succeed");

        let maintained_sql = format!(
            "SELECT d, user_id, dt, val, payload FROM {schema}.facts_enriched WHERE d = DATE \
             '2024-01-01'"
        );
        let oracle_sql = format!(
            "SELECT f.d, f.user_id, f.dt, f.val, dm.payload FROM {schema}.sources_facts f JOIN \
             {schema}.sources_dims dm ON f.user_id = dm.user_id AND f.dt = dm.dt WHERE f.d = \
             DATE '2024-01-01'"
        );
        assert!(
            multiset_equal_via_backend_with_diff(
                backend.as_ref(),
                &maintained_sql,
                &oracle_sql,
                |l, r| b.multiset_diff_sql(l, r)
            )
            .await
            .expect("composite-key multiset comparison"),
            "composite-key (user_id, dt) join fan-out diverged from the full-refresh oracle"
        );
    }
}

/// `hazard_schedules_are_pinned` — the twin of
/// `maintenance_conformance::pinned::hazard_schedules_are_pinned`, minus
/// `keyed_merge_reprocessed_window` (see this module's doc comment).
pub async fn run_hazard_schedules_are_pinned(b: &dyn ConformanceBackend) -> Result<()> {
    hazard::additive_agg_append_only_control(b).await;
    hazard::additive_agg_redelivery(b).await;
    hazard::idempotent_agg_append_only_control(b).await;
    hazard::join_enrichment_mutable_dimension(b).await;
    hazard::holistic_agg_append_only_control(b).await;
    hazard::composite_key_join_fan_out(b).await;
    Ok(())
}
