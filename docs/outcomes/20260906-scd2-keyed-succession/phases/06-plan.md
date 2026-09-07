# Phase 6 plan — Append-only probe: the succession late-append leg

## Objective

Close success criterion 7. The count-gated fingerprint leg is already in the tree (verified
below), so the deliverable is conformance evidence *for the succession grain*: a late append
landing in a closed **event-time** partition of a succession source must be classified as an
observation and its covering window re-presented, never raised as
`SourceMutationProfileViolated`. Paired with an in-place-mutation control on the same recipe so
a green late-append leg cannot be a probe that silently never dispatched.

## Pre-verified (no implementation work)

- `emit_append_only_posture_probe`'s fingerprint leg is count-gated in the tree:
  `AppendOnlyBaselinePartition::check_fingerprint` plus the emitted predicate
  `... AND __current.current_count = __baseline.recorded_count AND __current.current_fingerprint
  IS DISTINCT FROM __baseline.recorded_fingerprint`
  (`crates/smelt-logical/src/maintenance/emit/probes.rs:503,606`), and
  `dispatch_and_record_append_only_postures` classifies a closed partition's count increase via
  `late_appends` into a `tracing::warn!` + `ProbeRecord.observed`, never an error
  (`crates/smelt-runtime/src/source_probes.rs:289`). Record this in the phase summary; do not
  re-implement or re-test it at the unit level.
- The succession harness already renders `probes: { cadence: per_run }`
  (`render.rs:335`) and `mutation_profile: append_only` on the succession source
  (`render/succession.rs`), so both legs below exercise a live probe.

## Spec delta

None — no user-visible behaviour changes. This phase adds conformance coverage of behaviour
already specified in `docs/specs/model_properties.md` §Constraints "Declared lateness is
orchestration-only" and `docs/specs/sources.md` §Known Divergences (append-only probe
fingerprint leg). If the sources.md divergence bullet is now fully covered, leave its rewrite to
phase 10 (spec closure) rather than touching it here.

## Tests

All in `crates/smelt-cli/tests/maintenance_conformance/probes.rs`, using
`SuccessionRecipe::new_lead().with_delete_filter().with_source(
SourceRecipe::succession_events_event_time_partitioned())` and the existing
`gate_succession` quartet — no new testkit helper unless a leg genuinely cannot be expressed.

1. `succession_late_append_into_closed_partition_is_re_presented` — stage the recipe; drive
   window d1 then window d2 (each closing its own event-time partition, so a baseline is
   recorded for both); insert a *late* event whose `changed_at` falls inside the already-closed
   d1 partition; re-drive window d1. The run must succeed (no `SourceMutationProfileViolated`)
   and the maintained table must equal the full-refresh oracle over the current source —
   i.e. the covering window was re-presented and the late event spliced.
2. `succession_in_place_mutation_of_a_closed_partition_still_violates` — the non-vacuity
   control. Same recipe and the same first two windows, then `UPDATE` a staged row's payload in
   the closed d1 partition (row count unchanged), then drive window d1 via
   `drive_succession_window_expect_probe_failure`. The run must fail with a message naming
   `SourceMutationProfileViolated`, and both the presented table and the tombstone ledger must
   be byte-unchanged (the helper already asserts this).

Both legs are fixed-recipe pinned cases, not generated samples — the schedule they need
(close a partition, then append into it) is not one `arb_schedule_for` draws for the
succession family.

## Tasks

1. Read `probes.rs`'s `late_append_schedule_holds_with_probes_on` (line 664) as the shape to
   mirror, and `gate_succession::{drive_succession_window_and_assert_for,
   drive_succession_window_expect_probe_failure}` for the drive seams.
2. Write test 1 red-first: assert the run succeeds and equivalence holds. If it passes on the
   first run, prove non-vacuity via test 2 before accepting either.
3. Write test 2, driving the in-place `UPDATE` through `project.connect()` (the same seam
   `insert_row_succession_for` uses) — a raw payload rewrite at unchanged row count.
4. Give each test a doc comment naming the criterion (7) and the spec section it evidences,
   in the file's existing style.
5. If a leg cannot be expressed without a testkit widening, add the smallest helper to
   `crates/smelt-maintenance-testkit/src/gate_succession.rs` with its own unit test; do not
   weaken an assertion to make a leg pass — record the finding for the next planner instead.
6. Check `bash .claude/scripts/large-file-check.sh` before committing (probes.rs is 737 lines).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -20`
- `cargo test -p smelt-maintenance-testkit --quiet 2>&1 | tail -20` (only if task 5 fires)
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`test(succession): prove a late append into a closed event-time partition is re-presented`
