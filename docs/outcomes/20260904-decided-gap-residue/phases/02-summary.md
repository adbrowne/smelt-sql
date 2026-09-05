# Phase 2 summary — deferral oracle restated as landed-vs-processed

**Shipped:**
- `crates/smelt-logical/src/contract/deferral.rs`: `LagBoundViolation` + pure
  `settled_lag_bound(unprocessed_event_times, input_frontier, d)` — the
  restated obligation's lag-bound leg, reusing `settled_cutoff` (no second
  formula). Two new unit tests.
- `crates/smelt-logical/src/contract/mod.rs`: `OracleObligation::Bracketed`
  replaced by `ExactOverProcessedSWithLagBound`; `oracle_obligation` dispatch
  and module doc comments updated; `settled_cutoff`'s doc comment marks it as
  retained purely as the vacuity witness for the superseded bracket.
- `crates/smelt-maintenance-testkit/src/s_tracker.rs`: `record_landing`,
  `landed_at(k)`, `landed_not_processed(k)` — landings accumulate
  independently of `runs`, so `s_at`/`record_run` are untouched. `s_at_settled`
  kept, doc comment rewritten to name it superseded. Two new unit tests.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs`: the deferral arm
  of `assert_equivalence_at_point_with_frontier` now does strict
  `multiset_equal_via_backend` over `S` (leg 1, identical to the other two
  obligations) plus `deferral::settled_lag_bound` over
  `tracker.landed_not_processed(k)`'s event times (leg 2) — no bracket, no
  comparator logic of the gate's own.
- `crates/smelt-cli/tests/maintenance_conformance/contract_points.rs`: the
  deferral fixture (renamed
  `deferral_recipe_upholds_restated_oracle_with_a_skipped_run`) now records
  run B's rows via `record_landing`, not `record_run` — `deferred_model`
  never folded that window, so recording it as a run inflated `S` past what
  the model actually processed (the vacuity's root cause). New metamorphic
  test `deferral_comparator_rejects_a_state_the_bracket_admitted`: deletes
  every row from the maintained table after run A, shows both legs of the
  OLD bracket still pass (leg 1 trivially against an empty table; leg 2
  vacuously since the cutoff precedes all recorded event time), then asserts
  the restated comparator returns `Err`.
- `docs/TODO.md`: "Deferral oracle restatement" bullet removed.

**Decisions:**
- Named the vacuity precisely (recorded 2026-09-05 in outcome.md before this
  summary): the old bracket's lower leg (`full_refresh(S_settled) ⊆
  maintained`) holds vacuously whenever the settled cutoff precedes all
  recorded event time, so the upper leg alone admits any subset of
  `full_refresh(S)`.
- `s_at_settled`/`materialize_s_settled` are kept rather than deleted — the
  metamorphic test needs them to demonstrate the superseded bracket's own
  vacuity inline. Both are now dead outside that one test and `#[cfg(test)]`
  doc references; `#![allow(dead_code)]` at the top of `s_tracker.rs` already
  covers this.
- `landed_at(k)` takes a run index `k` even though landings themselves carry
  none: it unions all recorded landings (unconditionally) with every run
  snapshot up to and including `k`, so `landed_not_processed(k) = L \
  s_at(k)` stays well-defined at any point in a fixture's sequence.

**For the next planner:**
- Phase 3 (once-write fallback-case nullability route) is next per the
  outcome's phase table; untouched by this phase.
- No new residue surfaced — `oracle_obligation`'s three-way match stays
  exhaustive, and the standing `maintenance_conformance`/`statement_parity`
  gates both stay green with no other call site needing an update (grepped:
  `OracleObligation::Bracketed` had exactly one production call site, in
  `gate.rs`).

**Gates:**
- `cargo test -p smelt-logical --lib contract::` — 47 passed.
- `cargo test -p smelt-maintenance-testkit` — 59 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 79 passed.
- `cargo test -p smelt-runtime --test statement_parity` — 37 passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, full workspace `cargo test`, `example_diagnostics`).
