# Phase 2 plan — sub-`g_part` refusal names the coarsened run window

## Objective

Make every run-window refusal that stems from the partition grid tell the operator the exact
`--event-time-start`/`--event-time-end` pair that would be accepted, instead of only naming the
minimum unit. Advances success criterion 2, and closes the "does not yet name the coarsened
window" Known Divergence bullet in the same commit (phase-1 precedent: never ship a refusal
while the spec calls it unimplemented).

## Design (the reading this phase commits to)

There are two distinct refusals, and only one of them has an actionable window suggestion:

1. **Window-level** — the window's own bounds are not on `g_part` boundaries (today caught by
   `validate_run_window_alignment`'s granularity-boundary arms, plus the new
   `g_part`-grid check for the `g_run` coarser-than-`g_part` residue, e.g. `g_run = month`
   over `g_part = week`). Fix is a different window → the message names `g_part` and the
   coarsened pair, and re-running with that pair succeeds.
2. **Config-level** — `g_run < g_part` (declared `timeseries.granularity` finer than the
   derived grid). No window makes this pass; the fix is a frontmatter edit. The message names
   the config change *and*, as context, the covering window at `g_part` — phrased so it is not
   read as "re-run with this and it works".

Both messages format the pair through one shared helper, so the spelling cannot drift.

## Spec delta

`docs/specs/incremental_shapes.md`:
- §"Run window vs partition granularity" — replace the single trailing sentence about the
  sub-`g_part` rejection with the two-refusal statement above: the window-level refusal names
  the partition granularity and spells the coarsened
  `[--event-time-start, --event-time-end)` pair that would be accepted; the config-level
  `g_run < g_part` refusal names the required `granularity` value and the covering window at
  `g_part`. Auto-coarsening stays rejected.
- §Known Divergences → "### The partition grain" — delete the "**The sub-`g_part` rejection
  does not yet name the coarsened window**" bullet.

## Tests

Unit (`crates/smelt-runtime/src/windowing.rs` `#[cfg(test)]`, or the existing
`tests/windowing_parity.rs` where the alignment messages are already asserted):
- `coarsen_window_to_grid_floors_start_and_ceils_end` — month unit over `2024-12-05 →
  2024-12-20` yields `2024-12-01 → 2025-01-01`; an already-aligned pair is returned unchanged.
- `misaligned_run_window_names_the_coarsened_pair` — `validate_run_window_alignment` on a
  monthly model with `2024-12-05 → 2024-12-20` errors with text containing
  `--event-time-start 2024-12-01` and `--event-time-end 2025-01-01`.
- `window_off_the_partition_grid_is_refused_with_the_pair` — the `g_run = month`,
  `g_part = week` residue: `validate_run_window_against_partition_grid` refuses naming `week`
  and the week-coarsened pair (the case the `g_run >= g_part` comparison alone lets through).
- `sub_g_part_config_refusal_names_the_required_granularity` — `g_run = hour`,
  `g_part = day`: message names `day`, the `timeseries.granularity` fix, and the day-covering
  window; asserted not to claim the re-run succeeds.

End-to-end (`crates/smelt-runtime/tests/partition_grid_validation.rs`, real `execute_project`
against DuckDB — template the model's `DATE_TRUNC` unit alongside `{granularity}`):
- `test_misaligned_window_refusal_names_a_pair_that_then_succeeds` — monthly model, window
  `2024-12-05 → 2024-12-20` is refused; the test **parses the two dates out of the refusal
  message**, re-runs `execute_project` with exactly them, and asserts the run succeeds and the
  output rows are correct. This is criterion 2's "asserts the printed pair and that
  re-running with it succeeds".

## Tasks

1. Add `fn coarsen_window_to(start, end, unit) -> (NaiveDate, NaiveDate)` in `windowing.rs`,
   reusing `align_output_start`/`align_output_end`; add
   `fn suggested_window_flags(start, end) -> String` rendering
   `--event-time-start YYYY-MM-DD --event-time-end YYYY-MM-DD`.
2. Thread the model's effective grid unit into `validate_run_window_alignment`'s misalignment
   messages so each names the coarsened pair (keep the existing prefixes — current assertions
   use `contains`).
3. In `validate_run_window_against_partition_grid`, after the existing `g_run < g_part`
   comparison, add the window-vs-`g_part` grid check (refuse when the bounds are not
   `g_part`-aligned) with the pair-bearing message; reword the `g_run < g_part` message per
   the design above.
4. Write the unit tests, then the end-to-end test (template `{trunc}` in
   `REVENUE_MODEL_SQL`/`setup_project`); red before the code lands where practical.
5. Apply the spec delta; drop the ratchet entry in
   `crates/smelt-cli/tests/partition_residue_probes.rs` (`expected_leads` 3 → 2, message text
   updated) and fix that test's stale "the six this outcome does not own" doc comment flagged
   by the phase-1 summary.
6. Flip the phase-2 row to `done` and write `phases/02-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test partition_grid_validation`
- `cargo test -p smelt-runtime --test windowing_parity`
- `cargo test -p smelt-cli --test partition_residue_probes`
- `cargo test -p smelt-logical --test walk_coverage`

## Commit message

`feat(incremental): sub-g_part run-window refusals name the coarsened window pair`
