//! The keyed-succession (SCD2) family's conformance leg matrix
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/07b-plan.md`) —
//! phase 7a's two smoke cases plus the full deterministic leg matrix
//! criterion 6 enumerates: delete semantics, ordering/idempotence, the
//! pre-window clamp, an event-time-partitioned source, and the clock-tie
//! probe's rollback discriminator. Every leg is driven through the real
//! `execute_project` pipeline against DuckDB and asserted against the
//! model's own SQL at full refresh after every window.

use chrono::NaiveDate;

use smelt_maintenance_testkit::gate_succession::{
    drive_succession_window_and_assert_for, drive_succession_window_expect_probe_failure,
    insert_row_succession_for, stage_succession_recipe_for, SuccessionEventRow,
};
use smelt_maintenance_testkit::recipe::{ConformanceTarget, SourceRecipe, SuccessionRecipe};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

fn dt(y: i32, m: u32, d: u32, h: u32) -> chrono::NaiveDateTime {
    date(y, m, d).and_hms_opt(h, 0, 0).expect("valid time")
}

/// Snapshot `relation`'s full contents as a comparable, order-independent
/// value — this test module's counterpart of
/// `smelt_maintenance_testkit::gate_succession`'s private
/// `snapshot_relation_rows` (not reachable from this crate).
async fn snapshot_rows(
    project: &smelt_maintenance_testkit::link_c_harness::LinkCProject,
    relation: &str,
) -> Vec<std::collections::BTreeMap<String, String>> {
    let backend = project.backend().await.expect("backend");
    let batches = backend
        .execute_sql(&format!("SELECT * FROM {relation} ORDER BY ALL"))
        .await
        .unwrap_or_else(|e| panic!("snapshot {relation:?}: {e}"));
    smelt_runtime::check_runner::batches_to_rows(&batches)
}

fn tombstone_relation(model_name: &str) -> String {
    format!(
        "main.{}",
        smelt_logical::maintenance::emit::tombstone_table_name(model_name)
    )
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

/// Leg 1: `delete_event_then_later_insert_matches_oracle` — a `QUALIFY NOT
/// is_deleted` recipe: fold a delete for key 1, then a later non-delete
/// event for the same key; the neighbour chain must re-splice around the
/// tombstoned row.
#[tokio::test]
async fn delete_event_then_later_insert_matches_oracle() {
    let mut recipe = SuccessionRecipe::new_lead().with_delete_filter();
    recipe.model_name = "customer_history_delete_then_insert".to_string();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    let window1_rows = vec![
        SuccessionEventRow::new(1, dt(2026, 1, 1, 8), "gold"),
        SuccessionEventRow::deleted(1, dt(2026, 1, 2, 8), "gold"),
    ];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg1-1",
        date(2026, 1, 1),
        date(2026, 1, 3),
        &window1_rows,
    )
    .await
    .expect("window 1 (seed + delete) must succeed and match the oracle");

    let window2_rows = vec![SuccessionEventRow::new(1, dt(2026, 1, 3, 8), "silver")];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg1-2",
        date(2026, 1, 3),
        date(2026, 1, 4),
        &window2_rows,
    )
    .await
    .expect("window 2 (later insert) must re-splice around the tombstone and match the oracle");
}

/// Leg 2: `late_insert_before_a_folded_delete_matches_oracle` — a delete is
/// folded in window 1; window 2 lands a late insert whose `event_time`
/// precedes it, so the delete's neighbour must repatch.
#[tokio::test]
async fn late_insert_before_a_folded_delete_matches_oracle() {
    let mut recipe = SuccessionRecipe::new_lead().with_delete_filter();
    recipe.model_name = "customer_history_late_before_delete".to_string();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    let window1_rows = vec![
        SuccessionEventRow::new(1, dt(2026, 1, 1, 8), "gold"),
        SuccessionEventRow::deleted(1, dt(2026, 1, 3, 8), "gold"),
    ];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg2-1",
        date(2026, 1, 1),
        date(2026, 1, 4),
        &window1_rows,
    )
    .await
    .expect("window 1 (seed + delete) must succeed and match the oracle");

    // A late insert whose event_time (Jan 2) precedes the already-folded
    // delete (Jan 3), arriving only in window 2's own range.
    let window2_rows = vec![SuccessionEventRow::late(
        1,
        dt(2026, 1, 2, 8),
        "silver",
        date(2026, 1, 5),
    )];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg2-2",
        date(2026, 1, 5),
        date(2026, 1, 6),
        &window2_rows,
    )
    .await
    .expect("window 2 (late insert before the delete) must repatch and match the oracle");
}

/// Leg 3: `delete_only_key_is_absent_from_state_and_oracle` — a key whose
/// only events are deletes appears in neither the maintained table nor the
/// oracle (and the run still succeeds).
#[tokio::test]
async fn delete_only_key_is_absent_from_state_and_oracle() {
    let mut recipe = SuccessionRecipe::new_lead().with_delete_filter();
    recipe.model_name = "customer_history_delete_only_key".to_string();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    let window_rows = vec![SuccessionEventRow::deleted(9, dt(2026, 1, 1, 8), "gold")];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg3",
        date(2026, 1, 1),
        date(2026, 1, 2),
        &window_rows,
    )
    .await
    .expect("a delete-only key must still succeed and match the oracle");

    let rows = snapshot_rows(&project, &format!("main.{}", recipe.model_name)).await;
    assert!(
        rows.iter()
            .all(|r| r.get("customer_id").map(String::as_str) != Some("9")),
        "a delete-only key must not appear in the maintained table: {rows:#?}"
    );
}

/// Leg 4: `lag_projection_under_delete_and_late_splice_matches_oracle` —
/// legs 1-2's schedule over a `LAG` recipe with the delete filter on:
/// key 1 exercises "late insert before a folded delete", key 2 exercises
/// "delete then later insert".
#[tokio::test]
async fn lag_projection_under_delete_and_late_splice_matches_oracle() {
    let mut recipe = SuccessionRecipe::new_lag().with_delete_filter();
    recipe.model_name = "customer_history_lag_delete_late_splice".to_string();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    let window1_rows = vec![
        SuccessionEventRow::new(1, dt(2026, 1, 1, 8), "gold"),
        SuccessionEventRow::deleted(1, dt(2026, 1, 3, 8), "gold"),
        SuccessionEventRow::new(2, dt(2026, 1, 1, 8), "gold"),
        SuccessionEventRow::deleted(2, dt(2026, 1, 2, 8), "gold"),
    ];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg4-1",
        date(2026, 1, 1),
        date(2026, 1, 4),
        &window1_rows,
    )
    .await
    .expect("window 1 must succeed and match the oracle");

    let window2_rows = vec![
        // key 1: a late insert whose event_time precedes the folded delete.
        SuccessionEventRow::late(1, dt(2026, 1, 2, 8), "silver", date(2026, 1, 5)),
        // key 2: a later, non-delete event after the folded delete.
        SuccessionEventRow::new(2, dt(2026, 1, 5, 8), "bronze"),
    ];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg4-2",
        date(2026, 1, 5),
        date(2026, 1, 6),
        &window2_rows,
    )
    .await
    .expect("window 2 must succeed and match the oracle");
}

/// Leg 5: `windows_applied_out_of_order_converge` — two disjoint arrival
/// windows driven in reverse chronological order end at the same state the
/// oracle gives.
#[tokio::test]
async fn windows_applied_out_of_order_converge() {
    let mut recipe = SuccessionRecipe::new_lead();
    recipe.model_name = "customer_history_out_of_order".to_string();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    let window_b_rows = vec![SuccessionEventRow::new(2, dt(2026, 1, 5, 8), "silver")];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg5-b",
        date(2026, 1, 5),
        date(2026, 1, 6),
        &window_b_rows,
    )
    .await
    .expect("window B (driven first) must succeed and match the oracle");

    let window_a_rows = vec![SuccessionEventRow::new(1, dt(2026, 1, 1, 8), "gold")];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg5-a",
        date(2026, 1, 1),
        date(2026, 1, 2),
        &window_a_rows,
    )
    .await
    .expect("window A (driven second, out of order) must succeed and match the oracle");
}

/// Leg 6: `repeated_window_application_is_idempotent` — driving the SAME
/// window twice leaves both the presented table and the tombstone ledger
/// byte-identical (snapshot both around the second run).
#[tokio::test]
async fn repeated_window_application_is_idempotent() {
    let mut recipe = SuccessionRecipe::new_lead();
    recipe.model_name = "customer_history_repeated_window".to_string();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    let rows = vec![
        SuccessionEventRow::new(1, dt(2026, 1, 1, 8), "gold"),
        SuccessionEventRow::new(2, dt(2026, 1, 2, 8), "bronze"),
    ];
    for row in &rows {
        insert_row_succession_for(&project, &recipe, row).expect("insert window rows");
    }

    let mut request = smelt_maintenance_testkit::link_c_harness::base_request("dev");
    request.start = Some("2026-01-01".to_string());
    request.end = Some("2026-01-03".to_string());
    project
        .run_quiet("succession-leg6-1", request.clone())
        .await
        .expect("first fold must succeed");

    let presented = format!("main.{}", recipe.model_name);
    let tombstones = tombstone_relation(&recipe.model_name);
    let presented_before = snapshot_rows(&project, &presented).await;
    let tombstones_before = snapshot_rows(&project, &tombstones).await;

    project
        .run_quiet("succession-leg6-2", request)
        .await
        .expect("refolding the same window must succeed, not refuse");

    let presented_after = snapshot_rows(&project, &presented).await;
    let tombstones_after = snapshot_rows(&project, &tombstones).await;
    assert_eq!(
        presented_before, presented_after,
        "refolding the same window must leave the presented table byte-identical"
    );
    assert_eq!(
        tombstones_before, tombstones_after,
        "refolding the same window must leave the tombstone ledger byte-identical"
    );
}

/// Leg 7: `pre_window_clamp_excludes_clamped_rows_from_state_and_oracle` — a
/// recipe with `clamp: Some("changed_at >= TIMESTAMP '...'")`; rows below
/// the clamp are absent from both sides (asserted non-vacuously: at least
/// one inserted row is clamped away).
#[tokio::test]
async fn pre_window_clamp_excludes_clamped_rows_from_state_and_oracle() {
    let mut recipe =
        SuccessionRecipe::new_lead().with_clamp("changed_at >= TIMESTAMP '2026-01-02 00:00:00'");
    recipe.model_name = "customer_history_clamp".to_string();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    let rows = vec![
        // Below the clamp — must be excluded from both sides.
        SuccessionEventRow::new(1, dt(2026, 1, 1, 8), "gold"),
        // At/above the clamp — must be present.
        SuccessionEventRow::new(2, dt(2026, 1, 2, 8), "silver"),
    ];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg7",
        date(2026, 1, 1),
        date(2026, 1, 3),
        &rows,
    )
    .await
    .expect("run must succeed and match the (clamped) oracle");

    let maintained = snapshot_rows(&project, &format!("main.{}", recipe.model_name)).await;
    assert!(
        !maintained.is_empty(),
        "the clamp must not exclude every row (non-vacuous check)"
    );
    assert!(
        maintained
            .iter()
            .all(|r| r.get("customer_id").map(String::as_str) != Some("1")),
        "the clamped-away row (key 1) must be absent from the maintained table: {maintained:#?}"
    );
}

/// Leg 8: `event_time_partitioned_source_matches_oracle` —
/// `partition_column: None` (windows scan the clock itself); a two-window
/// schedule matches the oracle.
#[tokio::test]
async fn event_time_partitioned_source_matches_oracle() {
    let mut recipe = SuccessionRecipe::new_lead()
        .with_source(SourceRecipe::succession_events_event_time_partitioned());
    recipe.model_name = "customer_history_event_time_partitioned".to_string();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    let window1_rows = vec![SuccessionEventRow::new(1, dt(2026, 1, 1, 8), "gold")];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg8-1",
        date(2026, 1, 1),
        date(2026, 1, 2),
        &window1_rows,
    )
    .await
    .expect("window 1 must succeed and match the oracle");

    let window2_rows = vec![SuccessionEventRow::new(1, dt(2026, 1, 3, 8), "silver")];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg8-2",
        date(2026, 1, 3),
        date(2026, 1, 4),
        &window2_rows,
    )
    .await
    .expect("window 2 must succeed and match the oracle");
}

/// Leg 9: `equal_key_clock_collision_rolls_back_with_succession_clock_tie` —
/// a second, non-identical row at an already-folded `(k, t)` fails the run
/// with a message naming `SuccessionClockTie`, the key and the clock value,
/// and leaves the presented table AND the ledger unchanged.
#[tokio::test]
async fn equal_key_clock_collision_rolls_back_with_succession_clock_tie() {
    let mut recipe = SuccessionRecipe::new_lead();
    recipe.model_name = "customer_history_clock_tie".to_string();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    // Establish the tables with a clean first window over a different key.
    let seed_rows = vec![SuccessionEventRow::new(2, dt(2026, 1, 1, 8), "gold")];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg9-seed",
        date(2026, 1, 1),
        date(2026, 1, 2),
        &seed_rows,
    )
    .await
    .expect("seed window must succeed");

    // Two colliding rows for key 1 at the same (customer_id, changed_at)
    // with DIFFERENT payloads.
    let colliding_rows = vec![
        SuccessionEventRow::new(1, dt(2026, 1, 2, 8), "gold"),
        SuccessionEventRow::new(1, dt(2026, 1, 2, 8), "silver"),
    ];
    let message = drive_succession_window_expect_probe_failure(
        &project,
        &recipe,
        "succession-leg9-collision",
        date(2026, 1, 2),
        date(2026, 1, 3),
        &colliding_rows,
    )
    .await
    .expect("the colliding run must fail loud and leave both tables unchanged");

    assert!(
        message.contains("SuccessionClockTie"),
        "refusal must name SuccessionClockTie, got: {message}"
    );
    assert!(
        message.contains("customer_id"),
        "refusal must name the key column, got: {message}"
    );
    assert!(
        message.contains("changed_at"),
        "refusal must name the clock column, got: {message}"
    );
}

/// Leg 10: `identical_re_presented_row_is_a_no_op` — the same `(k, t)` row
/// re-presented byte-identically succeeds and changes nothing (the
/// discriminator that keeps leg 9 from being a blanket ban on redelivery).
#[tokio::test]
async fn identical_re_presented_row_is_a_no_op() {
    let mut recipe = SuccessionRecipe::new_lead();
    recipe.model_name = "customer_history_identical_redelivery".to_string();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
        .expect("stage succession recipe");

    let rows = vec![SuccessionEventRow::new(1, dt(2026, 1, 1, 8), "gold")];
    drive_succession_window_and_assert_for(
        &project,
        &recipe,
        "succession-leg10-1",
        date(2026, 1, 1),
        date(2026, 1, 2),
        &rows,
    )
    .await
    .expect("first fold must succeed");

    let presented = format!("main.{}", recipe.model_name);
    let tombstones = tombstone_relation(&recipe.model_name);
    let presented_before = snapshot_rows(&project, &presented).await;
    let tombstones_before = snapshot_rows(&project, &tombstones).await;

    // Re-present the exact same (key, clock, payload) row byte-identically —
    // a SECOND physical row, not a refold of window 1's own row (that's leg
    // 6). NOTE: driven directly (not via `drive_succession_window_and_assert_for`)
    // because the family quartet's oracle
    // (`render_succession_oracle_body_over`) is the model's raw SQL over the
    // raw physical source with no dedup — a genuine SECOND physical row at
    // the same `(k, t)` makes the naive oracle emit two (identical) output
    // rows where the MERGE-keyed presented table can only ever hold one, so
    // the state-unchanged snapshot check below is this leg's real assertion,
    // not oracle equivalence.
    insert_row_succession_for(
        &project,
        &recipe,
        &SuccessionEventRow::new(1, dt(2026, 1, 1, 8), "gold"),
    )
    .expect("insert the identical redelivery row");
    let mut request = smelt_maintenance_testkit::link_c_harness::base_request("dev");
    request.start = Some("2026-01-01".to_string());
    request.end = Some("2026-01-02".to_string());
    project
        .run_quiet("succession-leg10-2", request)
        .await
        .expect("an identical re-presented row must succeed as a no-op, not refuse");

    let presented_after = snapshot_rows(&project, &presented).await;
    let tombstones_after = snapshot_rows(&project, &tombstones).await;
    assert_eq!(
        presented_before, presented_after,
        "an identical re-presented row must leave the presented table byte-identical"
    );
    assert_eq!(
        tombstones_before, tombstones_after,
        "an identical re-presented row must leave the tombstone ledger byte-identical"
    );
}
