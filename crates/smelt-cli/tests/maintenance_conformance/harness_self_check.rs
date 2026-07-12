//! The harness-is-not-lying self-check
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 3;
//! pattern: `crates/smelt-db/tests/nullability_property_tests.rs`'s
//! seeded-divergence check). If the S-restricted oracle never actually
//! fails, `gate.rs`'s standing pass could just mean the oracle is
//! vacuously true — this test proves it isn't by directly corrupting a
//! maintained output row and asserting the oracle catches it.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::recipe::{arb_recipe, ConstructKind, RecipePool};
use smelt_maintenance_testkit::schedule_gen::arb_schedule_for;
use smelt_maintenance_testkit::verdict::{classify, Verdict};

use crate::gate::{assert_equivalence, drive_and_assert, stage_recipe};

/// `oracle_flags_a_seeded_divergence` (plan Phase 3 TDD list): after a
/// green run, directly corrupt one output row via a raw connection and
/// assert the oracle reports inequality.
#[test]
fn oracle_flags_a_seeded_divergence() {
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
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");

    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected the pinned additive-agg append-only recipe to admit: {verdict:?}"
    );

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (tracker, k) = rt
        .block_on(drive_and_assert(&project, &recipe, &schedule))
        .expect("green run must uphold equivalence before corruption");

    // Directly corrupt one maintained output row via a raw connection —
    // bypassing smelt's run pipeline entirely.
    {
        let conn = project.connect().expect("connect for corruption");
        conn.execute(
            &format!(
                "UPDATE main.{table} SET total = total + 999999 \
                 WHERE total = (SELECT MIN(total) FROM main.{table})",
                table = recipe.model_name,
            ),
            [],
        )
        .expect("seed a divergence");
    }

    let result = assert_equivalence(&project, &recipe, &tracker, k);
    assert!(
        result.is_err(),
        "oracle failed to flag a seeded divergence — the harness would be silently \
         reporting equivalence even when the maintained output is wrong"
    );
}
