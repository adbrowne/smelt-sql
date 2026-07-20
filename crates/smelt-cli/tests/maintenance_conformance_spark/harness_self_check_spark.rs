//! Spark twin of `maintenance_conformance/harness_self_check.rs`'s
//! harness-is-not-lying self-check (`docs/plans/20260720-prod-w9-spark-conformance-twin.md`
//! Phase 3). If the Spark S-restricted oracle never actually fails, the
//! Spark leg's standing pass could just mean the oracle is vacuously true on
//! this backend too — this test proves it isn't, by directly corrupting a
//! maintained Delta row (via `Backend::execute_sql`, never a raw
//! host-filesystem write) and asserting the oracle catches it.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::recipe::{
    arb_recipe, ConformanceTarget, ConstructKind, RecipePool, SPARK_CONFORMANCE_SCHEMA,
};
use smelt_maintenance_testkit::schedule_gen::arb_schedule_for;
use smelt_maintenance_testkit::verdict::{classify, Verdict};

use crate::gate_spark::{
    assert_equivalence_spark, drive_and_assert_spark, spark_connect_url, stage_recipe_spark,
};

/// `oracle_flags_a_seeded_divergence_on_spark` (plan Phase 3 TDD list): after
/// a green Spark run, directly corrupt one maintained output row via the
/// Backend trait and assert the S-restricted oracle reports inequality.
#[test]
fn oracle_flags_a_seeded_divergence_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!("SPARK_CONNECT_URL unset — skipping oracle_flags_a_seeded_divergence_on_spark");
        return;
    };

    // Pin to AdditiveAgg — `verdict.rs`'s own pinned test guarantees this
    // construct always admits `Technique::DeleteInsert` over an append-only
    // source, so this case is reliably green before corruption.
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();
    let schedule = arb_schedule_for(&recipe)
        .new_tree(&mut runner)
        .unwrap()
        .current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_spark(&recipe, &tmp).expect("stage spark recipe");

    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected the pinned additive-agg append-only recipe to admit: {verdict:?}"
    );

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (tracker, k) = rt
        .block_on(drive_and_assert_spark(&project, &recipe, &schedule))
        .expect("green Spark run must uphold equivalence before corruption");

    // Directly corrupt one maintained output row via the Backend trait —
    // bypassing smelt's run pipeline entirely, but never a raw host-path
    // write (spec's backend-client-API requirement).
    let backend = rt
        .block_on(project.backend_for_target(ConformanceTarget::SparkDelta))
        .expect("open backend for corruption");
    // Delta's `UPDATE` refuses a subquery in the SET/WHERE clause
    // (`DELTA_UNSUPPORTED_SUBQUERY`), unlike DuckDB's own self-check
    // (`harness_self_check.rs`'s `WHERE total = (SELECT MIN(total) ...)`) —
    // an unconditional whole-table bump needs no subquery and is just as
    // effective a seeded divergence (every row's `total` no longer matches
    // the oracle).
    rt.block_on(backend.execute_sql(&format!(
        "UPDATE {schema}.{table} SET total = total + 999999",
        schema = SPARK_CONFORMANCE_SCHEMA,
        table = recipe.model_name,
    )))
    .expect("seed a divergence on Spark");

    let result = rt.block_on(assert_equivalence_spark(
        &recipe,
        backend.as_ref(),
        &tracker,
        k,
    ));
    assert!(
        result.is_err(),
        "Spark oracle failed to flag a seeded divergence — the harness would be silently \
         reporting equivalence even when the maintained output is wrong"
    );
}
