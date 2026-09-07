# Phase 7b plan — Succession conformance leg matrix

## Objective

Turn phase 7a's two smoke cases into the full deterministic leg matrix criterion 6 enumerates,
driven through the real `execute_project` pipeline against DuckDB and asserted against the
model's own SQL at full refresh after every window. Advances **criterion 6** (conformance) and
gives criterion 5's runtime claims (refold idempotence, either-order convergence, clock-tie
rollback) their end-to-end proof. Cross-suite widening (`state_deletion.rs`, `repair.rs`, the
`deferral` leg) is phase 7c.

## Spec delta

None. This phase adds tests only; every behaviour it exercises is already normative in
`docs/specs/incremental_shapes.md` §"The succession grain" (§"Delete events", §"Run shape and
late events") and §"Succession-grain constraints". If a leg's expected end state turns out to
contradict the spec, stop and record it — do not adjust the oracle to match the code.

## Tests

All in `crates/smelt-cli/tests/maintenance_conformance/succession.rs` unless noted. Each drives
windows via `drive_succession_window_and_assert_for`, so the oracle comparison runs after
*every* window, not only at the end.

1. `delete_event_then_later_insert_matches_oracle` — a `QUALIFY NOT is_deleted` recipe: fold a
   delete for key 1, then a later non-delete event for the same key; the neighbour chain must
   re-splice around the tombstoned row.
2. `late_insert_before_a_folded_delete_matches_oracle` — a delete is folded in window 1; window
   2 lands a late insert whose `event_time` precedes it, so the delete's neighbour must repatch.
3. `delete_only_key_is_absent_from_state_and_oracle` — a key whose only events are deletes
   appears in neither the maintained table nor the oracle (and the run still succeeds).
4. `lag_projection_under_delete_and_late_splice_matches_oracle` — legs 1–2's schedule over a
   `LAG` recipe with the delete filter on.
5. `windows_applied_out_of_order_converge` — two disjoint arrival windows driven in reverse
   chronological order end at the same state the oracle gives.
6. `repeated_window_application_is_idempotent` — driving the *same* window twice leaves both the
   presented table and the tombstone ledger byte-identical (snapshot both around the second run).
7. `pre_window_clamp_excludes_clamped_rows_from_state_and_oracle` — a recipe with
   `clamp: Some("changed_at >= TIMESTAMP '…'")`; rows below the clamp are absent from both sides
   (asserted non-vacuously: at least one inserted row is clamped away).
8. `event_time_partitioned_source_matches_oracle` — `partition_column: None` (windows scan the
   clock itself); a two-window schedule matches the oracle.
9. `equal_key_clock_collision_rolls_back_with_succession_clock_tie` — a second, non-identical
   row at an already-folded `(k, t)` fails the run with a message naming `SuccessionClockTie`,
   the key and the clock value, and leaves the presented table *and* the ledger unchanged.
10. `identical_re_presented_row_is_a_no_op` — the same `(k, t)` row re-presented byte-identically
    succeeds and changes nothing (the discriminator that keeps leg 9 from being a blanket ban).
11. `succession_event_row_deleted_carries_the_flag` (testkit unit,
    `gate_succession.rs`) — `SuccessionEventRow::deleted*` sets `is_deleted`, and
    `insert_row_succession_for` omits the arrival column for an event-time-partitioned source.

## Tasks

1. Widen `SuccessionEventRow` in `crates/smelt-maintenance-testkit/src/gate_succession.rs` with
   `deleted(key, event_time, payload)` and `deleted_late(key, event_time, payload, arrival)`.
2. Make `insert_row_succession_for` derive its column list from the recipe's source
   (`partition_column` / `delete_flag_column` `Option`s) instead of assuming both are present —
   today it always emits an arrival value, which an event-time-partitioned source has no column
   for. Keep the `INSERT INTO … VALUES` shape.
3. Add `SourceRecipe::succession_events_event_time_partitioned()` in
   `crates/smelt-maintenance-testkit/src/recipe/succession.rs` (`partition_column: None`, delete
   flag retained) and small `SuccessionRecipe` combinators — `with_delete_filter()`,
   `with_clamp(pred)`, `with_source(src)` — rather than a new named constructor per leg. Each
   must keep `model_name` unique per variant so staged projects do not collide.
4. Add `drive_succession_window_expect_probe_failure(project, recipe, run_id, start, end, rows)
   -> Result<String>` to `gate_succession.rs`: snapshots the presented table and the tombstone
   ledger, drives the window expecting `run_quiet` to fail, returns the error message, and
   asserts both snapshots are unchanged. Reuse `crate::gate::snapshot_table_rows`' idiom; the
   ledger's table name follows the phase-1 `<presented table>__tombstones` pin.
5. Write legs 1–4 (delete semantics), then 5–6 (ordering/idempotence), then 7–8 (clamp,
   event-time partitioning), then 9–10 (clock tie), red-green one at a time.
6. Keep `succession.rs` under the large-file baseline — if it crosses, split the delete legs
   into `succession/deletes.rs` rather than raising the baseline.

## Verification

- `cargo test -p smelt-maintenance-testkit --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -20` (full seeded
  sample — the succession legs must not perturb the existing 83)
- `bash .claude/scripts/large-file-check.sh`
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`test(maintenance-testkit): add the succession conformance leg matrix`
