//! Spark twin of `maintenance_conformance/pinned.rs`
//! (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 5):
//! reproduces the same construct × posture catalogue coverage and hazard
//! schedules against a live Spark/Delta backend, reusing the same typed
//! recipe generators and the already-ported Spark drive helpers
//! (`gate_spark`/`gate_keyed_spark`/`gate_mixed_spark`) rather than
//! duplicating rendering logic.
//!
//! `hazard::keyed_merge_reprocessed_window` is the ONE hazard case NOT
//! ported: it specifically exercises `KeyedCombiner::Additive`'s
//! never-fold-twice REFUSAL, which requires the reconciliation ledger this
//! plan's "Deferred during implementation" entry already documents as
//! DuckDB-only (no Spark dialect for MP12's ledger DDL yet) — on Spark the
//! very first fold in that hazard's own schedule would fail loud with the
//! ledger's own `BackendError::unsupported` rather than the hazard's own
//! `KeyedReprocessedWindow` refusal, so the case does not reproduce
//! (there is no Spark equivalent to pin).

use chrono::NaiveDate;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{
    arb_recipe, BodyConstruct, ConformanceTarget, KeyedCombiner, KeyedRecipe, ModelRecipe,
    MutableEnrichedRecipe, RecipePool, SPARK_CONFORMANCE_SCHEMA,
};
use smelt_maintenance_testkit::schedule_gen::{ConformanceSchedule, ConformanceStep, GenRow};
use smelt_maintenance_testkit::verdict::{classify, classify_keyed, Verdict};

use crate::gate_keyed_spark::stage_keyed_recipe_spark;
use crate::gate_mixed_spark::{classify_mixed, insert_fact_row_spark, stage_mixed_recipe_spark};
use crate::gate_spark::{drive_and_assert_spark, spark_connect_url, stage_recipe_spark};

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("valid base date")
        .checked_add_signed(chrono::Duration::days(offset))
        .expect("valid offset date")
}

/// Copied from `pinned.rs::pinned_body_recipe` — pure generator draw, no
/// backend dependency, so identical regardless of target.
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

/// Copied from `pinned.rs::minimal_schedule` — pure data, no backend
/// dependency.
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

/// Spark twin of `pinned.rs::assert_pinned_body_recipe_is_green`.
fn assert_pinned_body_recipe_is_green_spark(recipe: &ModelRecipe) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_spark(recipe, &tmp)
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} failed to stage on Spark: {e}"));
    let verdict = classify(&project, recipe)
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} classify failed: {e}"));
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "pinned recipe {recipe:?} did not admit: {verdict:?}"
    );

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(drive_and_assert_spark(
        &project,
        recipe,
        &minimal_schedule(),
    ))
    .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} Spark equivalence check failed: {e}"));
}

/// `pinned_recipes_reproduce_catalogue_coverage_on_spark` (plan Phase 5 TDD
/// list): the Spark twin of
/// `maintenance_conformance::pinned::pinned_recipes_reproduce_catalogue_coverage`.
#[test]
fn pinned_recipes_reproduce_catalogue_coverage_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             pinned_recipes_reproduce_catalogue_coverage_on_spark"
        );
        return;
    };

    assert_pinned_body_recipe_is_green_spark(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::PassThrough)
    }));
    assert_pinned_body_recipe_is_green_spark(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::Filter { .. })
    }));
    assert_pinned_body_recipe_is_green_spark(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::AdditiveAgg)
    }));
    assert_pinned_body_recipe_is_green_spark(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::IdempotentAgg)
    }));
    assert_pinned_body_recipe_is_green_spark(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::DecomposedAgg)
    }));
    assert_pinned_body_recipe_is_green_spark(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::HolisticAgg)
    }));

    let mixed = MutableEnrichedRecipe::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_mixed_recipe_spark(&mixed, &tmp)
        .unwrap_or_else(|e| panic!("stage mixed recipe on Spark: {e}"));
    let plan = classify_mixed(&project, &mixed)
        .unwrap_or_else(|e| panic!("classify mixed recipe on Spark: {e}"));
    assert!(
        !plan.cells.is_empty(),
        "mutable-dimension-enrichment recipe admitted zero cells on Spark: {plan:#?}"
    );

    // Both `KeyedCombiner` families classify cleanly on Spark — admission is
    // pure classification, unaffected by the `Grade::Additive` ledger gap
    // that excludes execution of the Additive family elsewhere in this plan.
    for combiner in [KeyedCombiner::Additive, KeyedCombiner::Idempotent] {
        let recipe = KeyedRecipe::new_window_forward(combiner);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe_spark(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("stage keyed recipe {combiner:?} on Spark: {e}"));
        let verdict = classify_keyed(&project, &recipe)
            .unwrap_or_else(|e| panic!("classify keyed recipe {combiner:?} on Spark: {e}"));
        assert!(
            matches!(verdict, Verdict::Admitted(_)),
            "keyed recipe {combiner:?} did not admit on Spark: {verdict:?}"
        );
    }
}

/// The retired hazard cases ported to Spark (see this module's doc comment
/// for `keyed_merge_reprocessed_window`'s exclusion).
mod hazard {
    use super::*;

    pub fn additive_agg_append_only_control() {
        assert_pinned_body_recipe_is_green_spark(&pinned_body_recipe(|c| {
            matches!(c, BodyConstruct::AdditiveAgg)
        }));
    }

    pub fn additive_agg_redelivery() {
        let recipe = pinned_body_recipe(|c| matches!(c, BodyConstruct::AdditiveAgg));
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe_spark(&recipe, &tmp).expect("stage recipe on Spark");
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
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(drive_and_assert_spark(&project, &recipe, &schedule))
            .unwrap_or_else(|e| panic!("redelivery schedule Spark equivalence check failed: {e}"));
    }

    pub fn idempotent_agg_append_only_control() {
        assert_pinned_body_recipe_is_green_spark(&pinned_body_recipe(|c| {
            matches!(c, BodyConstruct::IdempotentAgg)
        }));
    }

    pub fn join_enrichment_mutable_dimension() {
        let recipe = MutableEnrichedRecipe::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_mixed_recipe_spark(&recipe, &tmp).expect("stage mixed recipe on Spark");
        let plan = classify_mixed(&project, &recipe).expect("classify mixed recipe on Spark");
        assert!(!plan.cells.is_empty(), "expected admission: {plan:#?}");

        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let backend = project
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open spark backend");

            insert_fact_row_spark(
                backend.as_ref(),
                &recipe,
                &GenRow {
                    d: day(0),
                    id: 1,
                    val: 10,
                },
            )
            .await
            .expect("insert fact row on Spark");
            let mut request = smelt_maintenance_testkit::link_c_harness::base_request("spark");
            request.start = Some("2024-01-01".to_string());
            request.end = Some("2024-01-02".to_string());
            project
                .run_with_target(
                    ConformanceTarget::SparkDelta,
                    "pinned-g05-spark-run-1",
                    request,
                    &smelt_runtime::NoOpReporter,
                )
                .await
                .expect("first Spark run");

            // Mutate the dimension in place — the exact hazard `G-05` named.
            backend
                .execute_sql(&format!(
                    "UPDATE {schema}.sources_{name} SET {attr} = 999 WHERE {key} = 1",
                    schema = SPARK_CONFORMANCE_SCHEMA,
                    name = recipe.dimension.name,
                    attr = recipe.dimension.payload_column,
                    key = recipe.dimension.key_column,
                ))
                .await
                .expect("mutate Spark dimension");

            // Re-run the SAME window: a catch-up run must reflect the
            // CURRENT dimension contents, not the stale one.
            let mut request = smelt_maintenance_testkit::link_c_harness::base_request("spark");
            request.start = Some("2024-01-01".to_string());
            request.end = Some("2024-01-02".to_string());
            project
                .run_with_target(
                    ConformanceTarget::SparkDelta,
                    "pinned-g05-spark-run-2",
                    request,
                    &smelt_runtime::NoOpReporter,
                )
                .await
                .expect("catch-up Spark run after dimension mutation");

            let maintained_sql = format!(
                "SELECT * FROM {SPARK_CONFORMANCE_SCHEMA}.{}",
                recipe.model_name
            );
            let oracle_sql = recipe
                .model_body()
                .replace(
                    &format!("smelt.sources.{}", recipe.dimension.name),
                    &format!(
                        "{SPARK_CONFORMANCE_SCHEMA}.sources_{}",
                        recipe.dimension.name
                    ),
                )
                .replace(
                    &format!("smelt.sources.{}", recipe.fact.name),
                    &format!("{SPARK_CONFORMANCE_SCHEMA}.sources_{}", recipe.fact.name),
                );
            assert!(
                multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql)
                    .await
                    .expect("catch-up multiset comparison on Spark"),
                "catch-up run after in-place dimension mutation diverged from a full-refresh \
                 oracle over the CURRENT dimension contents on Spark"
            );
        });
    }

    pub fn holistic_agg_append_only_control() {
        assert_pinned_body_recipe_is_green_spark(&pinned_body_recipe(|c| {
            matches!(c, BodyConstruct::HolisticAgg)
        }));
    }

    /// `g_10_composite_key_join_fan_out` ported to Spark/Delta: a real
    /// fact/dim project where `dims` is unique only on the PAIR
    /// `(user_id, dt)`. Self-contained (mirrors `pinned.rs`'s own
    /// self-staged project, targeting `SPARK_CONFORMANCE_SCHEMA` through
    /// `Backend::execute_sql` rather than a raw `duckdb::Connection`).
    pub fn composite_key_join_fan_out() {
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
        std::fs::write(
            project_dir.join("smelt.yml"),
            smelt_maintenance_testkit::render::render_smelt_yml_for(
                ConformanceTarget::SparkDelta,
                &db_path,
            ),
        )
        .expect("write smelt.yml");

        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let backend =
                smelt_maintenance_testkit::link_c_harness::open_spark_conformance_backend(&db_path)
                    .await
                    .expect("open spark backend");
            let schema = SPARK_CONFORMANCE_SCHEMA;
            for table in ["facts_enriched", "sources_facts", "sources_dims"] {
                backend
                    .execute_sql(&format!("DROP TABLE IF EXISTS {schema}.{table}"))
                    .await
                    .expect("drop stale g-10 table");
            }
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {schema}.sources_facts (d DATE, user_id INT, dt INT, val \
                     DOUBLE) USING DELTA"
                ))
                .await
                .expect("create g-10 facts table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {schema}.sources_facts VALUES (DATE '2024-01-01', 1, 100, \
                     10.0), (DATE '2024-01-01', 2, 200, 20.0)"
                ))
                .await
                .expect("seed g-10 facts");
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {schema}.sources_dims (user_id INT, dt INT, payload INT) \
                     USING DELTA"
                ))
                .await
                .expect("create g-10 dims table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {schema}.sources_dims VALUES (1, 100, 111), (1, 200, 222), \
                     (2, 100, 333), (2, 200, 444)"
                ))
                .await
                .expect("seed g-10 dims");

            let project =
                smelt_maintenance_testkit::link_c_harness::LinkCProject::load(project_dir, db_path)
                    .expect("load g-10 project");
            let mut request = smelt_maintenance_testkit::link_c_harness::base_request("spark");
            request.start = Some("2024-01-01".to_string());
            request.end = Some("2024-01-02".to_string());
            project
                .run_with_target(
                    ConformanceTarget::SparkDelta,
                    "pinned-g10-spark",
                    request,
                    &smelt_runtime::NoOpReporter,
                )
                .await
                .expect("g-10 Spark run must succeed");

            let maintained_sql = format!(
                "SELECT d, user_id, dt, val, payload FROM {schema}.facts_enriched WHERE d = \
                 DATE '2024-01-01'"
            );
            let oracle_sql = format!(
                "SELECT f.d, f.user_id, f.dt, f.val, dm.payload FROM {schema}.sources_facts f \
                 JOIN {schema}.sources_dims dm ON f.user_id = dm.user_id AND f.dt = dm.dt WHERE \
                 f.d = DATE '2024-01-01'"
            );
            assert!(
                multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql)
                    .await
                    .expect("composite-key multiset comparison on Spark"),
                "composite-key (user_id, dt) join fan-out diverged from the full-refresh oracle \
                 on Spark"
            );
        });
    }
}

/// `hazard_schedules_are_pinned_on_spark` (plan Phase 5 TDD list): the Spark
/// twin of `maintenance_conformance::pinned::hazard_schedules_are_pinned`,
/// minus `keyed_merge_reprocessed_window` (see this module's doc comment).
#[test]
fn hazard_schedules_are_pinned_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!("SPARK_CONNECT_URL unset — skipping hazard_schedules_are_pinned_on_spark");
        return;
    };

    hazard::additive_agg_append_only_control();
    hazard::additive_agg_redelivery();
    hazard::idempotent_agg_append_only_control();
    hazard::join_enrichment_mutable_dimension();
    hazard::holistic_agg_append_only_control();
    hazard::composite_key_join_fan_out();
}
