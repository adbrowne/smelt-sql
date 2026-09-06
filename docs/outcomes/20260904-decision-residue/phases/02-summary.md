# Phase 2 summary — sub-`g_part` refusal names the coarsened run window

**Shipped:**
- `windowing.rs`: `coarsen_window_to`, `suggested_window_flags`, `is_grid_aligned` helpers
  (private, reused by every refusal message so the rounding/formatting rule is stated once).
- `validate_run_window_alignment`'s every misalignment arm (Week/Month/Quarter/Year) now names
  the coarsened `[--event-time-start, --event-time-end)` pair via `coarsen_window_to`.
- `validate_run_window_against_partition_grid` now distinguishes two refusals: the existing
  `g_run < g_part` config-level refusal (reworded to name the required `timeseries.granularity`
  value, with the covering window at `g_part` as non-actionable context), and a new
  window-level refusal for the residue a bare `g_run >= g_part` comparison lets through — the
  window's own bounds not landing on `g_part` boundaries (e.g. monthly `g_run` over weekly
  `g_part`, since a month start isn't always a Monday).
- Spec (`incremental_shapes.md` §"Run window vs partition granularity") rewritten to describe
  both refusals; the "does not yet name the coarsened window" Known Divergence bullet deleted.
- `partition_residue_probes.rs` ratchet: `expected_leads` 3 → 2, stale "the six"/"seven" doc
  comment corrected to "the two this outcome does not own".

**Decisions:**
- Both new helpers reuse `align_output_start`/`align_output_end` rather than a second rounding
  implementation — the plan's stated goal ("one shared helper, so the spelling cannot drift")
  extended to *rounding*, not just formatting, since the skew-derived output window and a
  suggested run window must never disagree on what "coarsened to X" means.
- The config-level message explicitly disclaims that the covering window fixes anything
  ("will not make the run pass") rather than omitting the covering window entirely — named in
  the design as useful context, but only if it can't be misread as a fix.

**For the next planner:**
- No new residue surfaced. The e2e test
  (`test_misaligned_window_refusal_names_a_pair_that_then_succeeds`) proves the round-trip
  end-to-end through `execute_project`, closing criterion 2 completely.
- Not investigated: whether the `Week`-arm's three separate misalignment checks (start weekday,
  end weekday, total-days-not-a-multiple-of-7) could ever disagree on the suggested coarsened
  pair between arms — they can't (all three compute the same `coarsen_window_to`), but a future
  refactor merging those three checks should keep that property.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test partition_grid_validation` — 4/4 passed.
- `cargo test -p smelt-runtime --test windowing_parity` — 29/29 passed.
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb` — 4/4 passed.
- `cargo test -p smelt-logical --test walk_coverage` — 8/8 passed.
