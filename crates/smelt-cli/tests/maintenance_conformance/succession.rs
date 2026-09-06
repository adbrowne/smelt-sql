//! The keyed-succession (SCD2) family's own smoke conformance case
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/07a-plan.md` tests
//! 6/7) — the standing-gate proof that the testkit's succession scaffolding
//! (`smelt_maintenance_testkit::recipe::SuccessionRecipe`,
//! `smelt_maintenance_testkit::gate_succession`) works end to end through
//! the real `execute_project` pipeline before phase 7b widens it into a
//! full generated leg matrix.

use chrono::NaiveDate;

use smelt_maintenance_testkit::gate_succession::{
    drive_succession_window_and_assert_for, stage_succession_recipe_for, SuccessionEventRow,
};
use smelt_maintenance_testkit::recipe::{ConformanceTarget, SuccessionRecipe};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// `smoke_two_window_splice_matches_oracle` (phase 7a test 6): two windows,
/// the second inserting a LATE-arriving event for an already-seen key whose
/// `event_time` falls strictly between two events window 1 already folded —
/// the succession patch must re-splice the neighbour chain
/// (`valid_to`/`valid_from`) around the new arrival, matching the model
/// SQL's own full-refresh oracle after EVERY window.
#[tokio::test]
async fn smoke_two_window_splice_matches_oracle() {
    let recipe = SuccessionRecipe::new_lead();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    // Window 1: two events for key 1, both landing (arrival == event day)
    // inside [2026-01-01, 2026-01-03).
    let window1_rows = vec![
        SuccessionEventRow::new(1, date(2026, 1, 1).and_hms_opt(8, 0, 0).unwrap(), "gold"),
        SuccessionEventRow::new(1, date(2026, 1, 2).and_hms_opt(8, 0, 0).unwrap(), "bronze"),
    ];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-smoke-splice-1",
        date(2026, 1, 1),
        date(2026, 1, 3),
        &window1_rows,
    )
    .await
    .expect("window 1 (seed events) must succeed and match the oracle");

    // Window 2: a late-arriving event for the SAME key whose event_time
    // (2026-01-01 20:00) splices strictly between the two window-1 events,
    // but whose arrival (2026-01-03) lands only in window 2's own range.
    let window2_rows = vec![SuccessionEventRow::late(
        1,
        date(2026, 1, 1).and_hms_opt(20, 0, 0).unwrap(),
        "silver",
        date(2026, 1, 3),
    )];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-smoke-splice-2",
        date(2026, 1, 3),
        date(2026, 1, 4),
        &window2_rows,
    )
    .await
    .expect("window 2 (late splice) must succeed and match the oracle");
}

/// `smoke_lag_projection_matches_oracle` (phase 7a test 7): the same
/// two-window schedule shape, over a `LAG`-projecting recipe variant —
/// proves the renderer's `LAG` arm is exercised by the family quartet, not
/// just the `LEAD` arm test 6 covers.
#[tokio::test]
async fn smoke_lag_projection_matches_oracle() {
    let recipe = SuccessionRecipe::new_lag();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    let window1_rows = vec![
        SuccessionEventRow::new(1, date(2026, 1, 1).and_hms_opt(8, 0, 0).unwrap(), "gold"),
        SuccessionEventRow::new(2, date(2026, 1, 2).and_hms_opt(8, 0, 0).unwrap(), "bronze"),
    ];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-smoke-lag-1",
        date(2026, 1, 1),
        date(2026, 1, 3),
        &window1_rows,
    )
    .await
    .expect("window 1 must succeed and match the oracle");

    let window2_rows = vec![SuccessionEventRow::new(
        1,
        date(2026, 1, 3).and_hms_opt(8, 0, 0).unwrap(),
        "silver",
    )];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-smoke-lag-2",
        date(2026, 1, 3),
        date(2026, 1, 4),
        &window2_rows,
    )
    .await
    .expect("window 2 must succeed and match the oracle");
}
