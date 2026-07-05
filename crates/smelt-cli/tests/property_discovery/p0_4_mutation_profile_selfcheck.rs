//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `P0-4` (design §3b F7; plan phase C, acceptance (iii)).
//!
//! A generator's `MutationProfile` label (`AppendOnly` / `Mutable`) is a
//! *claim* about the shape of data it emits, never independently checked
//! against the schedule it actually produced. An unverified claim silently
//! poisons a verdict: if `arb_schedule` (declared `AppendOnly`) ever slipped
//! in an `InPlaceUpdate`/`InPlaceDelete` step, an `AppendOnly` cell would
//! secretly be exercising the mutable-snapshot hazard and any HOLDS verdict
//! for it would be meaningless. This module is the self-check that catches
//! that class of poisoning, proven on both the positive path (a real
//! violation is detected) and the negative path (the two shipped generators
//! actually honor their declared profile).

use chrono::NaiveDate;
use proptest::prelude::*;

use crate::run_schedule::{
    arb_mutable_schedule, arb_schedule, check_profile, MutationProfile, RunSchedule, ScheduleStep,
};

fn base_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
}

#[test]
fn self_check_detects_an_in_place_update_mislabeled_append_only() {
    // Red case: hand-construct a schedule containing an InPlaceUpdate step
    // and declare it AppendOnly — the self-check must refuse to rubber-stamp
    // this, or the whole point of P0-4 (an unverified label silently poisons
    // a verdict) is unaddressed.
    let schedule = RunSchedule(vec![
        ScheduleStep::AdvanceWindowAndRun {
            start: base_date(),
            end: base_date() + chrono::Duration::days(1),
        },
        ScheduleStep::InPlaceUpdate {
            id: 1,
            new_val: 42.0,
        },
    ]);

    let result = check_profile(&schedule, MutationProfile::AppendOnly);
    assert!(
        result.is_err(),
        "an InPlaceUpdate step must fail an AppendOnly profile check"
    );
    let (idx, _step) = result.unwrap_err();
    assert_eq!(idx, 1, "violation must point at the offending step index");
}

#[test]
fn self_check_detects_an_in_place_delete_mislabeled_append_only() {
    let schedule = RunSchedule(vec![
        ScheduleStep::AdvanceWindowAndRun {
            start: base_date(),
            end: base_date() + chrono::Duration::days(1),
        },
        ScheduleStep::InPlaceDelete { id: 1 },
    ]);

    assert!(
        check_profile(&schedule, MutationProfile::AppendOnly).is_err(),
        "an InPlaceDelete step must fail an AppendOnly profile check"
    );
}

#[test]
fn mutation_steps_are_permitted_under_the_mutable_profile() {
    // Same offending steps, but declared Mutable — a Mutable-profile cell is
    // supposed to see these; the check must not reject them.
    let schedule = RunSchedule(vec![
        ScheduleStep::AdvanceWindowAndRun {
            start: base_date(),
            end: base_date() + chrono::Duration::days(1),
        },
        ScheduleStep::InPlaceUpdate {
            id: 1,
            new_val: 42.0,
        },
        ScheduleStep::InPlaceDelete { id: 2 },
    ]);

    assert!(
        check_profile(&schedule, MutationProfile::Mutable).is_ok(),
        "InPlaceUpdate/InPlaceDelete steps are permitted under Mutable"
    );
}

proptest! {
    /// Green case: every schedule `arb_schedule` produces (declared
    /// `AppendOnly` — it only ever emits `AdvanceWindowAndRun`/
    /// `AppendLateRow` steps) must pass the `AppendOnly` self-check. This is
    /// the "does the existing AppendOnly generator actually honor its own
    /// label" half of the self-check.
    #[test]
    fn arb_schedule_output_always_matches_its_declared_append_only_profile(
        schedule in arb_schedule(base_date(), 1000)
    ) {
        prop_assert!(
            check_profile(&schedule, MutationProfile::AppendOnly).is_ok(),
            "arb_schedule is declared AppendOnly but emitted a schedule that \
             fails the self-check: {schedule:?}"
        );
    }

    /// Green case for the `Mutable` generator: every schedule it produces
    /// passes the (permissive) `Mutable` check, AND — the actual
    /// shape-verification half of F7 — contains at least one mutation step,
    /// so a `Mutable`-declared cell is provably exercising the mutation
    /// hazard rather than silently degenerating into an append-only run.
    #[test]
    fn arb_mutable_schedule_output_matches_profile_and_actually_mutates(
        schedule in arb_mutable_schedule(base_date(), 1000)
    ) {
        prop_assert!(
            check_profile(&schedule, MutationProfile::Mutable).is_ok(),
            "arb_mutable_schedule output failed its own (permissive) profile \
             check: {schedule:?}"
        );
        let has_mutation = schedule.0.iter().any(|s| {
            matches!(
                s,
                ScheduleStep::InPlaceUpdate { .. } | ScheduleStep::InPlaceDelete { .. }
            )
        });
        prop_assert!(
            has_mutation,
            "arb_mutable_schedule is declared Mutable but produced a schedule \
             with zero mutation steps — the declared shape was never \
             exercised: {schedule:?}"
        );
    }
}
