//! Trimmed-retention conformance pool (phase 8 of `docs/outcomes/
//! 20260906-trimmed-history-sources`): the partition-grain append-only
//! recipe pool ([`RecipePool::partition_append_only`]) with a declared
//! rolling `retention:` bound on the source, driven through the real
//! `execute_project` pipeline while the source is physically trimmed before
//! each run step (`smelt_maintenance_testkit::retention`).
//!
//! The declared bound and the schedule's own clock are anchored so that
//! `smelt-runtime`'s run-time admission check (which ages a run's window
//! against the REAL wall clock — `docs/outcomes/
//! 20260906-trimmed-history-sources/phases/05-plan.md`) never refuses a
//! forward run here: every window in [`arb_retention_schedule`] lands in the
//! future relative to `Utc::now()`, so `window_age` saturates at zero and a
//! forward run is always admitted regardless of calendar drift. The
//! declared bound is narrow relative to the SCHEDULE's own clock, though —
//! [`WINDOW_GAP_DAYS`] exceeds [`RETENTION_DAYS`] — so
//! [`trim_source_to_retention`] still physically departs an earlier
//! window's rows by the time a later window's own step runs. These two
//! clocks are deliberately independent (`outcome.md`'s phase 8 planning
//! decision log): the oracle equivalence check never depends on which rows
//! physically remain (`STracker` records each run's own snapshot at the
//! time it ran), so admission staying green and rows physically departing
//! are both true at once.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::recipe::{arb_payload_value, arb_recipe, ModelRecipe, RecipePool};
use smelt_maintenance_testkit::schedule_gen::{
    read_source_snapshot, ConformanceSchedule, ConformanceStep, GenRow,
};
use smelt_maintenance_testkit::verdict::{classify, Verdict};

use super::partition_pool::{
    assert_equivalence_with_edit, drive_and_assert_collecting, stage_recipe,
};

/// The declared retention bound every recipe in this pool uses (days).
const RETENTION_DAYS: i64 = 30;

/// The gap between successive windows in [`arb_retention_schedule`] (days) —
/// wider than [`RETENTION_DAYS`] so an earlier window's rows are guaranteed
/// to have departed the source (relative to the schedule's own clock) by
/// the time a later window's own run executes.
const WINDOW_GAP_DAYS: i64 = RETENTION_DAYS * 2;

/// Default deterministic case count, mirroring
/// [`super::partition_pool::DEFAULT_CASES`].
const DEFAULT_CASES: usize = 12;

pub(crate) fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// [`arb_recipe`] over the append-only partition pool, with a declared
/// `retention: '{RETENTION_DAYS} days'` bound on the source.
fn arb_retention_recipe() -> impl Strategy<Value = ModelRecipe> {
    arb_recipe(RecipePool::partition_append_only()).prop_map(|mut recipe| {
        recipe.source = recipe.source.with_retention(RETENTION_DAYS);
        recipe
    })
}

/// 2-3 forward `RunWindow` steps, [`WINDOW_GAP_DAYS`] apart, anchored a year
/// into the future relative to the real wall clock (see the module doc
/// comment for why: it decouples run-time admission from calendar drift and
/// from the retention bound that governs physical trimming).
fn arb_retention_schedule() -> impl Strategy<Value = ConformanceSchedule> {
    let anchor = chrono::Utc::now().date_naive() + chrono::Duration::days(365);
    (2_usize..=3).prop_flat_map(move |n_windows| {
        proptest::collection::vec(
            proptest::collection::vec(arb_payload_value(), 1..=2),
            n_windows,
        )
        .prop_map(move |window_vals| {
            let mut steps = Vec::new();
            let mut next_id = 1_i64;
            for (i, vals) in window_vals.iter().enumerate() {
                let start = anchor + chrono::Duration::days(i as i64 * WINDOW_GAP_DAYS);
                let end = start + chrono::Duration::days(1);
                let rows = vals
                    .iter()
                    .map(|val| {
                        let row = GenRow::new(start, next_id, *val);
                        next_id += 1;
                        row
                    })
                    .collect();
                steps.push(ConformanceStep::RunWindow { start, end, rows });
            }
            ConformanceSchedule(steps)
        })
    })
}

/// Total number of rows a [`ConformanceSchedule`] built by
/// [`arb_retention_schedule`] inserts over its lifetime (every step here is
/// a `RunWindow`, so this is just the sum of each step's own `rows`).
fn total_rows_inserted(schedule: &ConformanceSchedule) -> usize {
    schedule
        .0
        .iter()
        .map(|step| match step {
            ConformanceStep::RunWindow { rows, .. } => rows.len(),
            _ => 0,
        })
        .sum()
}

/// One case of the deterministic sample: stage `recipe`, classify it,
/// and — if admitted — drive `schedule` through the real pipeline
/// (asserting S-restricted equivalence after every step, same as
/// [`super::partition_pool`]'s own gate), returning whether the case was
/// admitted and, if so, whether at least one row physically departed the
/// source (`total_rows_inserted` vs. what [`read_source_snapshot`] finds
/// still present afterwards). Shared by both `#[test]`s below so a case is
/// only ever staged and driven once.
fn run_case(i: usize, recipe: &ModelRecipe, schedule: &ConformanceSchedule) -> Option<bool> {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(recipe, &tmp)
        .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));

    let verdict = classify(&project, recipe)
        .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));

    match verdict {
        Verdict::Refused(_) => None,
        Verdict::Admitted(_) => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            let mut migrate_outcomes = Vec::new();
            rt.block_on(drive_and_assert_collecting(
                &project,
                recipe,
                schedule,
                &mut migrate_outcomes,
            ))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: recipe {recipe:?} schedule {schedule:?} \
                     equivalence check failed under an advancing retention bound: {e}"
                )
            });

            let remaining = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.source).len()
            };
            Some(remaining < total_rows_inserted(schedule))
        }
    }
}

/// `retention_pool_upholds_equivalence_under_an_advancing_bound` (phase 8
/// TDD list): deterministic-seeded sample of retention-bearing recipes
/// driven through the real pipeline — S-restricted multiset equivalence
/// holds after every run step even though rows have physically departed the
/// source.
#[test]
fn retention_pool_upholds_equivalence_under_an_advancing_bound() {
    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_retention_recipe();
    let schedule_strat = arb_retention_schedule();

    let mut admitted_cases = 0;
    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        if run_case(i, &recipe, &schedule).is_some() {
            admitted_cases += 1;
        }
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic sample admitted zero cases — generator/derivation regression"
    );
}

/// `retention_pool_actually_trims_rows` (phase 8 TDD list, anti-vacuity —
/// mirrors `admission_rate_stays_above_floor`): at least one row must have
/// physically departed the source over the deterministic sample, else the
/// equivalence leg above asserts nothing about the no-silent-under-read
/// behaviour it exists to cover.
#[test]
fn retention_pool_actually_trims_rows() {
    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_retention_recipe();
    let schedule_strat = arb_retention_schedule();

    let mut any_row_departed = false;
    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        if let Some(departed) = run_case(i, &recipe, &schedule) {
            any_row_departed |= departed;
        }
    }

    assert!(
        any_row_departed,
        "N={n} deterministic sample never departed a single row — the retention bound \
         ({RETENTION_DAYS} days) or window gap ({WINDOW_GAP_DAYS} days) needs widening, this \
         leg asserts nothing about the no-silent-under-read behaviour otherwise"
    );
}

/// `an_aged_backfill_past_the_advancing_bound_refuses_and_leaves_state_unchanged`
/// (phase 8 TDD list, pinned/non-generative): after a source's declared
/// bound has — from the real run clock's perspective — long since advanced
/// past an old region, a `BackfillRegion` over it refuses
/// (`SourceRetentionExceeded`) before any statement executes, and the
/// maintained table still equals the oracle over the region it already
/// processed (the no-silent-under-read assertion at pipeline level).
#[test]
fn an_aged_backfill_past_the_advancing_bound_refuses_and_leaves_state_unchanged() {
    let recipe = arb_retention_recipe()
        .new_tree(&mut TestRunner::deterministic())
        .unwrap()
        .current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project =
        stage_recipe(&recipe, &tmp).unwrap_or_else(|e| panic!("failed to stage recipe: {e}"));

    let verdict = classify(&project, &recipe).unwrap_or_else(|e| panic!("classify failed: {e}"));
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "recipe {recipe:?} was refused at classification: {verdict:?}"
    );

    // A single forward run, safely in the future relative to the real wall
    // clock (age saturates at zero — see the module doc comment), so it
    // establishes a settled point unaffected by the declared retention
    // bound.
    let anchor = chrono::Utc::now().date_naive() + chrono::Duration::days(365);
    let good_schedule = ConformanceSchedule(vec![ConformanceStep::RunWindow {
        start: anchor,
        end: anchor + chrono::Duration::days(1),
        rows: vec![GenRow::new(anchor, 1, 5)],
    }]);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut migrate_outcomes = Vec::new();
    let (tracker, last_k) = rt
        .block_on(drive_and_assert_collecting(
            &project,
            &recipe,
            &good_schedule,
            &mut migrate_outcomes,
        ))
        .expect("the initial forward run must succeed");

    // Now attempt a backfill of a region deep in the real past — far older
    // than `RETENTION_DAYS` when aged against the REAL wall clock (unlike
    // this pool's forward `RunWindow` steps, a backfill's window start is
    // exactly what `smelt-runtime`'s admission check ages).
    let aged_start = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).expect("valid date");
    let aged_end = aged_start + chrono::Duration::days(1);
    let mut request = base_request("dev");
    request.start = Some(aged_start.format("%Y-%m-%d").to_string());
    request.end = Some(aged_end.format("%Y-%m-%d").to_string());

    let result = rt.block_on(project.run_quiet("aged-backfill", request));
    let err = result.expect_err(
        "a backfill reaching past the declared retention bound must refuse, not silently run",
    );
    assert!(
        err.to_string().contains("SourceRetentionExceeded"),
        "expected a SourceRetentionExceeded refusal, got: {err}"
    );

    // State is untouched by the refused backfill — the maintained table
    // still equals the oracle at the last good run.
    rt.block_on(assert_equivalence_with_edit(
        &project, &recipe, &tracker, last_k, None,
    ))
    .expect("maintained state must be unchanged after the refused backfill");
}
