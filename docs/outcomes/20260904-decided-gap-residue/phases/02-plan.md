# Phase 2 plan — restate the deferral oracle as landed-vs-processed

## Objective

Replace the conformance gate's `OracleObligation::Bracketed` deferral comparator with the
spec's restated landed-vs-processed form, single-owned in `smelt-logical`, and prove with a
metamorphic test that a deliberately wrong incremental state the bracket admitted is now
rejected. Advances success criterion 2.

## Why the bracket is vacuous (the finding this phase acts on)

`incremental_models.md` §"The contract lattice" (Deferral) states two obligations: equivalence
over the **processed** set stays strict — `incremental_state(S) == full_refresh(S)` — and every
input in `L \ S` (landed, not yet processed) arrived within `D`. The gate instead asserts
`full_refresh(S_settled) ⊆ maintained ⊆ full_refresh(S)`. Whenever the settled cutoff falls
before all recorded event time, `S_settled` is empty and the lower leg holds vacuously, so the
upper leg alone admits *any subset* of `full_refresh(S)` — including an empty output table.

The bracket only looked necessary because `STracker` conflates landed with processed: the
current fixture calls `record_run` for a window the deferred model never folded, which inflates
`S` past what the model processed. Splitting landing from processing is what lets the restated
(strict-over-S) form hold on a legitimately deferred model.

## Spec delta

None. `incremental_models.md` §"The contract lattice" already states the restated form; the
comparator is behind the spec, and no user-visible behaviour changes. Doc comments on the
changed functions must cite that paragraph rather than the superseded bracket wording.

## Tests

- `smelt-logical`, `contract::deferral`:
  - `settled_landed_input_must_be_processed` — the new pure predicate refuses an unprocessed
    landed event time strictly before `settled_cutoff(input_frontier, d)`, naming the offender.
  - `unsettled_landed_input_is_admitted` — event times at or after the cutoff, and an empty
    unprocessed set, are admitted.
- `smelt-logical`, `contract` point tests:
  - `deferral_obligation_is_exact_over_processed_s_with_lag_bound` — replaces
    `deferral_point_is_bracketed_and_does_not_restrict_the_window`; asserts the new obligation
    variant and that `restrict_run_window` is still the identity for deferral.
- `smelt-maintenance-testkit`, `s_tracker`:
  - `record_landing_does_not_advance_the_processed_set` — a landing leaves `s_at(k)` unchanged.
  - `landed_at_includes_rows_no_run_window_covered` — `L` contains landing-only rows and every
    run snapshot's rows; `landed_not_processed(k)` is exactly `L \ s_at(k)`.
- `smelt-cli`, `maintenance_conformance::contract_points`:
  - `deferral_recipe_upholds_restated_oracle_with_a_skipped_run` — the rewritten fixture (run B
    and the licensed skip in run C are recorded as *landings*, not runs); the restated
    comparator holds at every step, and the `skipped_deferral` manifest assertions are kept.
  - `deferral_comparator_rejects_a_state_the_bracket_admitted` — metamorphic. After run A,
    delete every row from `main.deferred_model`. Assert both legs of the *old* bracket still
    hold inline (`maintained EXCEPT ALL full_refresh(S)` is 0 rows, and `s_at_settled(...)` is
    empty so its leg was vacuous), then assert `assert_equivalence_at_point_with_frontier`
    returns `Err` under the restated comparator.

## Tasks

1. Add to `crates/smelt-logical/src/contract/deferral.rs` the pure lag-bound predicate over the
   unprocessed landed event times, e.g. `settled_lag_bound(unprocessed_event_times: &[i64],
   input_frontier: i64, d: i64) -> Result<(), LagBoundViolation>`, where `LagBoundViolation`
   names the earliest offending event time, the cutoff, and `d`. It reuses `settled_cutoff` —
   no second formula.
2. In `contract/mod.rs`, replace `OracleObligation::Bracketed` with
   `ExactOverProcessedSWithLagBound` and update `oracle_obligation`'s dispatch plus the module
   doc comment. Keep `settled_cutoff(point, …)`; `s_at_settled` stays (the metamorphic test
   uses it to demonstrate the vacuity), with its doc comment rewritten to say it is the
   *superseded* bracket leg retained only as the vacuity witness.
3. In `crates/smelt-maintenance-testkit/src/s_tracker.rs`, add `record_landing(snapshot)` (rows
   became visible in the source; this model folded nothing), `landed_at(k)`, and
   `landed_not_processed(k)`. Landings accumulate in their own list and never take a run index,
   so existing `s_at`/`record_run` semantics are untouched.
4. Rewrite `gate.rs`'s `assert_equivalence_at_point_with_frontier` deferral arm: leg 1 is the
   same both-direction `multiset_equal_via_backend` against `full_refresh(s_at_for_point(k))`
   the exact obligations already use; leg 2 feeds `tracker.landed_not_processed(k)`'s event
   times to `deferral::settled_lag_bound` and bails with its violation. The gate encodes no
   comparator of its own.
5. Update `contract_points.rs`'s deferral fixture per the tests above and add the metamorphic
   test; export whatever `gate.rs` helper the metamorphic test needs (`except_all_row_count_via_backend`).
6. Remove the "Deferral oracle restatement" bullet from `docs/TODO.md`.
7. Write `phases/02-summary.md` and flip the row to `done`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical contract`
- `cargo test -p smelt-maintenance-testkit`
- `cargo test -p smelt-cli --test maintenance_conformance` (the standing equivalence gate)
- `cargo test -p smelt-runtime --test statement_parity`

## Commit message

`test(contract): restate the deferral oracle as exact-over-processed with a landed lag bound`
