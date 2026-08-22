//! The harness-is-not-lying self-check family — the target-parametrized
//! twin of `maintenance_conformance/harness_self_check.rs`. If the
//! S-restricted oracle never actually fails, a leg's standing pass could
//! just mean the oracle is vacuously true on that backend too — this proves
//! it isn't, by directly corrupting a maintained row (via
//! `Backend::execute_sql`, never a raw host-filesystem write) and asserting
//! the oracle catches it.

use anyhow::Result;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use crate::families::gate::{assert_equivalence_for, drive_and_assert_for, stage_recipe_for};
use crate::families::ConformanceBackend;
use crate::recipe::{arb_recipe, ConstructKind, RecipePool};
use crate::verdict::{classify, Verdict};

/// `oracle_flags_a_seeded_divergence` — the twin of
/// `maintenance_conformance::harness_self_check::oracle_flags_a_seeded_divergence`:
/// after a green run, directly corrupt one maintained output row via the
/// Backend trait and assert the S-restricted oracle reports inequality.
pub async fn run_oracle_flags_a_seeded_divergence(b: &dyn ConformanceBackend) -> Result<()> {
    // Pin to AdditiveAgg — `verdict.rs`'s own pinned test guarantees this
    // construct always admits `Technique::DeleteInsert` over an append-only
    // source, so this case is reliably green before corruption.
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();
    let schedule = crate::schedule_gen::arb_schedule_for(&recipe)
        .new_tree(&mut runner)
        .unwrap()
        .current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let target = b.target(0);
    let schema = b.schema(0);
    let project = stage_recipe_for(&recipe, &tmp, target.clone()).expect("stage recipe");

    let verdict = classify(&project, &recipe).expect("classify");
    anyhow::ensure!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected the pinned additive-agg append-only recipe to admit: {verdict:?}"
    );

    let (tracker, k) = drive_and_assert_for(b, 0, &project, &recipe, &schedule)
        .await
        .expect("green run must uphold equivalence before corruption");

    // Directly corrupt one maintained output row via the Backend trait —
    // bypassing smelt's run pipeline entirely, but never a raw host-path
    // write.
    let backend = project
        .backend_for_target(target)
        .await
        .expect("open backend for corruption");
    backend
        .execute_sql(&b.corrupt_sql(0, &recipe))
        .await
        .expect("seed a divergence");

    let result = assert_equivalence_for(b, &schema, &recipe, backend.as_ref(), &tracker, k).await;
    anyhow::ensure!(
        result.is_err(),
        "oracle failed to flag a seeded divergence — the harness would be silently reporting \
         equivalence even when the maintained output is wrong"
    );
    Ok(())
}
