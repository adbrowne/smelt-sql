//! Conformance-gate leg: `.smelt/` deleted between run steps
//! (`docs/outcomes/20260904-state-residency/phases/09-plan.md`). The
//! outcome's headline claim made executable — deleting `.smelt/` never
//! changes what a maintained model computes, because the reconciliation
//! ledger and every other correctness structure now live in the engine, not
//! on disk. Reuses the existing public staging + drive helpers over the
//! partition and keyed pools (`gate.rs`) rather than duplicating either
//! drive loop; the only new behaviour is
//! `LinkCProject::with_state_deletion(StateDeletion::BetweenRuns)`.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::link_c_harness::StateDeletion;
use smelt_maintenance_testkit::recipe::{
    arb_keyed_combiner, arb_keyed_schedule, arb_recipe, KeyedRecipe, RecipePool,
};
use smelt_maintenance_testkit::schedule_gen::arb_schedule_for;
use smelt_maintenance_testkit::verdict::{classify, Verdict};

use crate::gate::{
    classify_keyed, drive_and_assert, drive_keyed_and_assert, stage_keyed_recipe, stage_recipe,
};

/// Default deterministic case count per pool — small (plan: "3 per pool")
/// since this leg drives every run step through a real `execute_project`
/// call AND a filesystem delete; `SMELT_STATE_DELETION_CASES` env override
/// for a deeper local sweep.
const DEFAULT_CASES: usize = 3;

fn case_count() -> usize {
    std::env::var("SMELT_STATE_DELETION_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// `partition_pool_upholds_equivalence_with_state_deleted` (plan test 2):
/// the append-only partition pool still upholds S-restricted multiset
/// equivalence with `.smelt/` removed before every run.
#[test]
fn partition_pool_upholds_equivalence_with_state_deleted() {
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
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"))
            .with_state_deletion(StateDeletion::BetweenRuns);

        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));

        match verdict {
            Verdict::Refused(_) => continue,
            Verdict::Admitted(_) => {
                admitted_cases += 1;
                rt.block_on(drive_and_assert(&project, &recipe, &schedule))
                    .unwrap_or_else(|e| {
                        panic!(
                            "case {i}: recipe {recipe:?} schedule {schedule:?} equivalence \
                             check failed with .smelt/ deleted between runs: {e}"
                        )
                    });
            }
        }

        assert!(
            project.nonempty_deletions_observed() > 0,
            "case {i}: recipe {recipe:?} never observed a non-empty .smelt/ deletion — the \
             leg is not exercising state-residency for this case"
        );
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic sample admitted zero cases — generator/derivation regression"
    );
}

/// `keyed_pool_upholds_end_state_equivalence_with_state_deleted` (plan test
/// 3): the keyed pool — whose never-fold-twice check runs against the
/// engine-resident `_smelt_ledger` — still upholds end-state equivalence
/// with `.smelt/` removed before every run. A green result here is the
/// end-to-end proof of criterion 1 (the ledger's fold survives because it
/// is engine-resident, not file-resident).
#[test]
fn keyed_pool_upholds_end_state_equivalence_with_state_deleted() {
    let n = case_count();
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
            .unwrap_or_else(|e| panic!("case {i}: keyed recipe {recipe:?} failed to stage: {e}"))
            .with_state_deletion(StateDeletion::BetweenRuns);

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
                     failed with .smelt/ deleted between runs: {e}"
                )
            });

        assert!(
            project.nonempty_deletions_observed() > 0,
            "case {i}: keyed recipe {recipe:?} never observed a non-empty .smelt/ deletion — \
             the leg is not exercising state-residency for this case"
        );
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic keyed sample admitted zero cases — generator/derivation regression"
    );
}

/// `deletion_leg_is_not_vacuous` (plan test 4): anti-vacuity — after driving
/// a schedule with `StateDeletion::BetweenRuns`, the harness's deletion
/// counter is > 0 and every counted deletion removed a directory that
/// existed and was non-empty. Locks the leg against silently degrading into
/// "delete nothing, assert equivalence".
#[test]
fn deletion_leg_is_not_vacuous() {
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    // The deterministic sequence's first admitted case is enough — this
    // test's whole point is the counter arithmetic, not broad coverage.
    for _ in 0..20 {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let schedule = arb_schedule_for(&recipe)
            .new_tree(&mut runner)
            .unwrap()
            .current();

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe(&recipe, &tmp)
            .expect("stage recipe")
            .with_state_deletion(StateDeletion::BetweenRuns);

        let verdict = classify(&project, &recipe).expect("classify");
        if matches!(verdict, Verdict::Refused(_)) {
            continue;
        }

        rt.block_on(drive_and_assert(&project, &recipe, &schedule))
            .expect("equivalence check with .smelt/ deleted between runs");

        assert!(
            project.deletions_observed() > 0,
            "deletion counter is zero — the deletion leg never actually removed a .smelt/ dir"
        );
        assert_eq!(
            project.nonempty_deletions_observed(),
            project.deletions_observed(),
            "every counted deletion should have removed a non-empty .smelt/ dir (a schedule \
             with a RunWindow step always populates state.mode: intervals manifests)"
        );
        return;
    }

    panic!(
        "deterministic sample admitted zero cases in 20 draws — generator/derivation regression"
    );
}
