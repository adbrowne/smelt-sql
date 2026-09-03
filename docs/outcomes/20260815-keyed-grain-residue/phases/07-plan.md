# Phase 7 plan — Make the non-DuckDB `Grade::Idempotent` ledger skip a recorded, visible fact

## Objective

Phase 2 landed the re-run-tolerant frontier record but skipped it on non-DuckDB dialects behind a
bare `tracing::warn!` — a silent omission of a state structure, which conflicts with `CLAUDE.md`
§"Fail-loud discipline". Turn that skip into a first-class, user-visible, run-recorded fact
carried on the existing `RunReporter` channel. This is the "minimum fix regardless of option"
named in the phase-3 blocked entry; it serves criterion 6 (standing gates green, no self-inflicted
residue) and does **not** touch phase 3's deferred product decision — the skip stays a skip, it
just stops being silent.

## Spec delta (made by the implement step, first)

- `docs/specs/incremental_shapes.md` §"The transactional frontier write (merge ledger)": after the
  "written automatically whenever the project's state mode supports it" sentence, state that where
  the backend offers no ledger substrate the re-run-tolerant bookkeeping record is **not written
  and the omission is reported as a named fact on the run's reporter channel** (never silently
  dropped), while the additive grade refuses the run outright there. Keep it timeless — no phase
  or plan vocabulary.
- `docs/specs/incremental_shapes.md` §Known Divergences → key grain: adjust/extend the existing
  ledger-substrate wording only if it currently implies the skip is silent. Do **not** add or
  reword anything that pre-judges phase 3's blocked decision.
- `docs-site/docs/reference/state.md` §"The reconciliation ledger": one sentence, user-facing —
  on a backend with no ledger substrate the bookkeeping record is skipped and the run says so.

## Tests (red-green)

- `crates/smelt-runtime/src/maintenance_driver.rs` (test module) —
  `idempotent_ledger_skip_on_non_duckdb_is_reported`: drive `run_windowed_keyed_maintenance` with a
  `RecordingBackend { dialect: SqlDialect::SparkSQL }`, an unsuppressed idempotent rule, and a
  capturing `RunReporter` supplied through `RetryPolicy`; assert the run **succeeds** and the
  reporter received exactly one state-structure-unavailable event naming the model, the dialect,
  the `merge ledger` structure, and that no ledger statement was recorded by the backend.
- `crates/smelt-runtime/src/maintenance_driver.rs` —
  `idempotent_ledger_on_duckdb_reports_no_unavailability`: same shape on `SqlDialect::DuckDB`;
  the ledger statements are recorded and the reporter sees zero such events (negative direction).
- `crates/smelt-runtime/src/execute.rs` (test module) —
  `buffered_state_structure_unavailable_replays_to_reporter`: the new `ReporterEvent` variant
  pushed onto an `EventSink` replays onto the downstream reporter with the right run/model, so a
  `--jobs > 1` run does not lose the fact.

## Tasks

1. Add `RunReporter::state_structure_unavailable(&self, run_id, model, structure, dialect, consequence)`
   as a defaulted no-op method in `crates/smelt-runtime/src/reporter.rs`, doc-commented with the
   spec anchor and the rule that a skipped state structure is always reported.
2. Add the matching `ReporterEvent::StateStructureUnavailable { .. }` variant in
   `crates/smelt-runtime/src/execute.rs`, its `EventSink` impl arm, and its `replay` match arm.
3. In `maintenance_driver.rs`'s `Grade::Idempotent` arm, replace the `tracing::warn!` else-branch
   with a `retry.reporter.state_structure_unavailable(retry.run_id, model_name, …)` call (the
   reporter, run id and model name are already in scope via `RetryPolicy`); update the surrounding
   block comment so it no longer says "skipped with a warning".
4. Implement the event in `crates/smelt-cli/src/reporter.rs` (`CliReporter`) as a `warn!` line
   naming model, structure, dialect and consequence, so it is visible in ordinary `smelt run`
   output; add the pass-through in `smelt-ui`'s `BroadcastReporter` only if that reporter forwards
   events verbatim — otherwise leave the default.
5. Make the spec + docs-site edits above.
6. Grep for any other silent `tracing::warn!`-only skip of a *state structure* in
   `maintenance_driver.rs`; if one exists, note it in the summary — do not widen this phase.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --lib maintenance_driver 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test keyed_frontier_bookkeeping --test statement_parity --test execute_parity 2>&1 | tail -20`
- `cargo test -p smelt-cli --test maintenance_conformance 2>&1 | tail -20`
- `cargo check -p smelt-maintenance-testkit --features spark,bigquery --all-targets` (the phase-6 guard)

## Commit message

`feat(runtime): report the skipped merge-ledger bookkeeping record instead of warning silently`
