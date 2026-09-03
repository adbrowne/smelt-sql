# Phase 16 summary — Observed-delta consumption, write side

**Shipped:**
- The change-suppressed keyed fold (`maintenance_driver::run_windowed_keyed_maintenance`,
  `Grade::Idempotent` merge arm, `create_group.is_none()`) now records its observed output delta
  in the same backend transaction as the write, via a new
  `WindowedKeyedRule::observed_delta_changed_keys_sql` trait method (default `None`, mirroring
  `recurrence_probe_sql`'s fail-closed shape) implemented on `CumulativeClassification`
  (`crates/smelt-runtime/src/cumulative.rs`). `Grade::Additive` cells and `Unconditional` writes
  record nothing (documented, not silently dropped).
- New `maintenance_driver::keyed_fold_changed_keys_select` / `keyed_fold_changed_row_predicate`
  (pub): the changed-key query for a keyed fold compares the target's stored value against the
  FOLD's own combine expression (`target.c IS DISTINCT FROM (<fold_expr>)`), not the raw delta
  column — genuinely different from the column-scoped MERGE's guard. `changed_keys_select`
  refactored to share its join/key-projection shape (`changed_keys_select_over_predicate`) with
  the new keyed-fold variant, parameterized by candidate alias and predicate text.
- `execute_staged_membership_recompute` now always records its observed delta (it is only ever
  reached in its conditional/suppressed form — an `Unconditional` verdict has no lowering and is
  skipped upstream) via a new `staged_candidate_changed_keys_select`: the union of new/changed
  keys (candidate vs. target) and departed keys (target vs. candidate anti-join, matching the
  emitter's own `delete_departed` statement). Threaded a `window: &PartitionRange` through the
  function and its one call site in `execute.rs`, built the same `start_date`/`end_date` way the
  neighbouring column-scoped call site already does.
- Both new write paths refuse fail-loud on a non-DuckDB backend (same posture as the existing
  column-scoped `Suppressed` arm) rather than silently writing without recording.
- `smelt_logical::maintenance::locality`: new `SettledEmptyVerdict` (`SettledNoOp` /
  `EmptyUnsettled` / `Dirty`) and pure `settled_empty_verdict(bound, window_end, now,
  delta_is_empty)` — the settle-bound × observed-delta composition's first live leg. Added
  `chrono` as a `smelt-logical` dependency for date/duration arithmetic (previously date-string
  parsing lived only in `smelt-runtime`).
- `propagation::plan_since_upstream_with_observed_deltas` gained a `now: &str` parameter and now
  consults the origin's own derived `SettleBound` (threaded through a new
  `ClampAndLocality::key_locality_settle_bound`, populated alongside `key_locality_slice` from the
  SAME `MaintenancePlan::key_locality` — never re-derived) on the present-and-empty arm, appending
  a `"<source>: recorded delta is empty for [start, end) — settled no-op (behind the settle
  bound)"` / `"... — empty this run (not yet settled)"` line to the dirty-set report. This is a
  reporting distinction only — the scheduled run set (`plan.runs`) is identical either way, proven
  by a dedicated assertion in the new test. `plan_since_upstream` (empty-lookup wrapper) passes
  `""` since the present-and-empty arm is unreachable with an empty lookup.
- `docs/specs/incremental_models.md` §"Observed deltas on model edges": extended to name the three
  recording write families, the keyed-fold fold-expression-vs-raw-column distinction, the
  departed-key clause for staged-candidate, the non-DuckDB write-side refusal, and the settled/
  unsettled reporting composition. "Observed-delta consumption is partial" Known Divergences
  bullet removed. `docs-site/docs/guide/incremental-models.md` extended with "Which writes record
  a delta" and "Settled vs. unsettled empty" paragraphs.

**Decisions:**
- `keyed_fold_changed_row_predicate`/`staged_candidate_changed_keys_select`/`changed_row_predicate`
  stay outside `smelt_logical::maintenance::emit`'s single-owner rule (D1's precedent, already
  established for the column-scoped predicate): observed-delta recording is smelt-state
  bookkeeping, not emitter-authored maintenance-statement text. Kept from drifting off the write's
  own guard by a dedicated cross-check test (`statement_parity.rs::
  keyed_fold_changed_key_select_matches_the_merge_guard`), the same discipline the column-scoped
  predicate already has.
- The observed-delta record's `model` key for the keyed fold uses `table` (the physical table
  name), not `model_name` — matching the existing column-scoped precedent
  (`execute_column_scoped_write_with_observed_delta` records under `table`, and the two can differ
  at the call site), so the read side's existing lookup-by-table-name convention isn't disturbed.
- `staged_candidate_changed_keys_select`'s departed-key leg always reports `NULL` for
  `delta_partition` — a departed key has no candidate row to read a partition value from, matching
  the write's own inability to name one for a row it no longer has any relation over.

**For the next planner:**
- `Grade::Additive` (ledger-interleaved) keyed folds still record nothing — documented in-code
  (`run_windowed_keyed_maintenance`'s doc comment) and in the spec body ("An unconditional write of
  any family never records one" — additive cells are a further, separately-noted gap since they
  never reach the `Grade::Idempotent` arm at all). If this needs closing, it belongs to phase 19's
  proof-residue sweep per the outcome's own decision log (2026-09-03, phase 16 planning), not a new
  row here.
- No other residue surfaced. The phase's own scope (write-side recording + the settle-bound
  reporting leg) is fully closed; success criterion 13 is met.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --lib maintenance::locality` — 32 passed (4 new
  `settled_empty_verdict_*` tests).
- `cargo test -p smelt-runtime --test observed_delta --test since_upstream_propagation --test
  statement_parity` — 58 passed (7 new keyed-fold/staged-candidate tests in `observed_delta.rs`,
  1 new cross-check test in `statement_parity.rs`, the existing settle-bound composition test
  extended in `since_upstream_propagation.rs`).
- `cargo test -p smelt-cli --features duckdb --test since_upstream --test maintenance_conformance`
  — 86 passed.
