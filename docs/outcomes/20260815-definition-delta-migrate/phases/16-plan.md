# Phase 16 plan — Observed-delta consumption, write side

## Objective

Make the two write families that still record nothing — the change-suppressed **keyed fold**
(`run_windowed_keyed_maintenance` under `WriteSuppression::Suppressed`) and the
**staged-candidate conditional recompute** (`execute_staged_membership_recompute`) — record their
observed output delta in the same backend transaction as the write, exactly as the column-scoped
conditional MERGE already does. Then give the settle-bound × observed-delta composition its first
live leg: an empty recorded delta whose window lies provably behind the derived settle bound is
reported as a *settled* no-op, not merely an empty-this-run one. Advances success criterion 15
(dispatch distinguishes what actually happened) and closes the "Observed-delta consumption is
partial" divergence.

## Spec delta (made first)

- `docs/specs/incremental_models.md` §"The graph layer" — "Observed deltas on model edges": name
  the three write families that record a delta (column-scoped conditional MERGE, change-suppressed
  keyed fold, staged-candidate conditional recompute) and state that an unconditional write never
  records one; add one sentence that an empty recorded delta whose window lies behind the model's
  derived settle bound is a *settled* no-op (provably final), distinct from an unsettled empty one,
  and that this distinction is reported, not a further pruning of work.
- `docs/specs/incremental_models.md` §Known Divergences: delete the "Observed-delta consumption is
  partial" bullet (both write-side clauses and the settle-composition clause close here); if the
  non-DuckDB write-side refusal is worth stating, it belongs in the §"Observed deltas" paragraph as
  behaviour, not as a divergence.

## Tests (red-green, in this order)

1. `smelt-logical` `maintenance::locality` unit — `settled_empty_verdict_is_settled_behind_the_bound`:
   route-1 `SettleBound::After`, window end + bound < now → `SettledNoOp`.
2. same module — `settled_empty_verdict_is_unsettled_inside_the_bound`: window still within the
   bound → `EmptyUnsettled`.
3. same module — `settled_empty_verdict_never_settles_on_route2`: `SettleBound::Never` → always
   `EmptyUnsettled`, never `SettledNoOp` (no sentinel horizon).
4. same module — `settled_empty_verdict_is_dirty_for_a_non_empty_delta`: non-empty delta →
   `Dirty` regardless of the bound.
5. `crates/smelt-runtime/tests/observed_delta.rs::keyed_fold_suppressed_records_changed_keys` — a
   suppressed keyed-fold step over 3-of-N changed rows records exactly those keys under
   `(model, step.range.start, step.range.end)`.
6. `..::keyed_fold_fully_suppressed_records_an_empty_delta` — present-and-empty, distinct from
   absent (no row at all before the run).
7. `..::keyed_fold_unconditional_records_no_delta` — an `Unconditional` verdict leaves the table
   untouched (the record is a byproduct of the conditional write, never derived after the fact).
8. `..::keyed_fold_delta_rolls_back_with_a_failed_write` — injected write failure leaves no delta
   row (same-transaction proof, mirroring the column-scoped test at
   `smelt-backend-duckdb/src/lib.rs::test_record_observed_delta_rolls_back_record_on_write_failure`).
9. `..::keyed_fold_suppressed_recording_refuses_a_non_duckdb_backend` — fail-loud, same posture as
   `execute_column_scoped_write_with_observed_delta`.
10. `..::staged_membership_recompute_records_changed_keys` — the staged-candidate recompute records
    the keys whose applied effect was not the identity, keyed on the run window.
11. `..::staged_membership_recompute_records_an_empty_delta_when_nothing_changed`.
12. `crates/smelt-runtime/tests/statement_parity.rs::keyed_fold_changed_key_select_matches_the_merge_guard`
    — the changed-key `SELECT`'s `IS DISTINCT FROM` predicate is byte-identical to the one
    `emit_keyed_fold_suppressed` guards its matched arm with (one comparison, two consumers).
13. `crates/smelt-runtime/tests/since_upstream_propagation.rs::empty_delta_behind_the_settle_bound_reports_a_settled_horizon`
    — the `--since-upstream` report names the skip as settled for a route-1 origin whose window is
    behind the bound, and as empty-this-run otherwise; the scheduled run set is identical in both
    cases (this leg reports a horizon, it does not prune further).

## Tasks

1. Spec delta above (§"Observed deltas on model edges" + Known Divergences bullet removal).
2. `smelt-logical/src/maintenance/locality.rs`: add `SettledEmptyVerdict { SettledNoOp,
   EmptyUnsettled, Dirty }` and the pure `settled_empty_verdict(&SettleBound, window_end: &str,
   now: &str, delta_is_empty: bool)`; unit tests 1–4. Single owner — no consumer re-derives it.
3. `maintenance_driver.rs`: add `WindowedKeyedRule::observed_delta_changed_keys_sql(schema, table,
   delta_sql, compared_columns, partition_column) -> Option<String>` (default `None`, mirroring
   `recurrence_probe_sql`'s fail-closed shape) so the rule supplies its own `unique_key`; implement
   it on `CumulativeClassification` via the existing `changed_keys_select`.
4. `run_windowed_keyed_maintenance`: in the `Grade::Idempotent` merge arm, when `suppression` is
   `Suppressed` and `create_group.is_none()`, route the merge through
   `Backend::execute_conditional_write_and_record_observed_delta` with the observed-delta ensure/
   upsert SQL keyed `(model_name, step.range.start, step.range.end)` and
   `step.range.column` as the partition projection; refuse non-DuckDB by name. `Grade::Additive`
   (ledger-folded) is out of this phase's reach — leave it recording nothing and say so in a doc
   comment. Tests 5–9.
5. `execute_staged_membership_recompute`: take a `window: &PartitionRange` and, when the emitted
   group is the conditional (suppressed) form, record the changed keys transactionally via the same
   backend hook; thread the window from `execute.rs`'s membership-recompute call site using the
   same `start_date`/`end_date` → `PartitionRange` construction the column-scoped call site at
   `execute.rs:2566` already uses. Tests 10–11.
6. `statement_parity.rs`: cross-check leg (test 12) beside the existing column-scoped one.
7. `propagation.rs::plan_since_upstream_with_observed_deltas`: take the run's `now` and the origin's
   derived `SettleBound` (already on `MaintenancePlan::key_locality`, carried through
   `derive_clamp_and_locality` — read it, never re-derive), call `settled_empty_verdict` on the
   present-and-empty arm, and render the two no-op reasons distinctly in the report. Test 13.
8. `docs-site/docs/guide/incremental-models.md`: extend the "What a recorded delta narrows"
   paragraph with which writes record and the settled-vs-empty distinction.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib maintenance::locality`
- `cargo test -p smelt-runtime --test observed_delta --test since_upstream_propagation --test statement_parity`
- `cargo test -p smelt-cli --features duckdb --test since_upstream --test maintenance_conformance`

## Commit

`feat(propagation): record observed deltas on the suppressed keyed fold and staged-candidate recompute; compose the settle bound with the empty-delta no-op`
