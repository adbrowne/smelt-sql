# Phase 32b — posture-derived key departure, runtime half

## Objective

Make the default contract point actually behave as the default: a snapshot-reconcile keyed run
deletes keys present in the target but absent from the incoming scan, in the same transaction as
the merge, and suppresses that delete exactly where phase 32's `retain_departed` point is
declared (dispatching its probe instead). Advances success criterion 9 (standing gates green,
`statement_parity` extended) and the divergence-closure criteria: it removes the runtime residue
of `incremental_models.md`'s "Posture-derived key departure" bullet and `incremental_shapes.md`'s
"Departed keys are still retained under snapshot-reconcile" bullet.

## Spec delta

Behaviour already matches what both specs *normatively* say (`incremental_shapes.md` §"Departed
keys and deletion": "The reconcile write deletes it — an anti-join of stored keys against the
scanned snapshot, executed in the same transaction as the merge"), so this is a
divergence-closure edit, not a surface change:

- `docs/specs/incremental_models.md` §Known Divergences — delete the "Posture-derived key
  departure's runtime half is unimplemented" bullet outright (declaration half landed in 32,
  runtime half lands here).
- `docs/specs/incremental_shapes.md` §Known Divergences (~line 1242) — delete the "Departed keys
  are still retained under snapshot-reconcile" bullet, including its "the opt-in retention
  relaxation … is likewise unbuilt" clause.
- Bump both files' `last_reviewed` to the implement date.
- Fix the now-stale carve-out prose in `crates/smelt-runtime/src/cumulative.rs`'s
  `execute_snapshot_reconcile` doc comment ("`emit_keyed_fold`'s shape carries no `DELETE` … the
  documented carve-out") and the "runtime half has not landed" paragraph in
  `crates/smelt-logical/src/contract/retain_departed.rs` / `contract/mod.rs`.

## Tests

1. `smelt-logical` unit (`maintenance/emit.rs`) `emit_departed_key_delete_shape` — the anti-join
   `DELETE` renders `NOT EXISTS` over the delta select with null-safe key equality, per dialect
   (`IS NOT DISTINCT FROM`, `<=>` on Spark), multi-column key included.
2. `smelt-logical` unit (`contract/retain_departed.rs`) `reconcile_disposition_ladder` — absent
   declaration → `Delete`; `Bool(true)` → `Retain`; `Bool(false)` → `Delete`; `Tombstone{col}` →
   `Retain` carrying the column.
3. `smelt-runtime` integration (new `tests/departed_key_reconcile.rs`, live DuckDB, precedent
   `tests/web_analytics_session_delta_restriction.rs`):
   `snapshot_reconcile_deletes_departed_key` — run a keyed snapshot-reconcile model, drop a key
   from the source, re-run; the stored table no longer carries the key and is multiset-equal to a
   full refresh of the new source.
4. Same file `snapshot_reconcile_retains_departed_key_when_declared` — identical fixture plus
   `contract: retain_departed: true`; the departed key survives and no `DELETE` is executed.
5. Same file `retain_departed_probe_is_dispatched_pre_write` — with the point declared, the
   probe emitted by `emit_departed_key_probe` runs before the merge and its retained-departed
   count reaches the run's probe records; with the tombstone form and an unmarked departed key,
   the probe reports a violation.
6. `smelt-runtime` `tests/statement_parity.rs` `snapshot_reconcile_delete_leg_parity` — the SQL
   the reconcile run executes is byte-identical to `emit_keyed_fold` + `emit_departed_key_delete`
   called directly, arrives as one `transactional: true` `StatementGroup`, and the post-run table
   is multiset-equal to a full refresh.

## Tasks

1. Add `emit_departed_key_delete(schema_table, key, delta_select, dialect) -> MaintenanceStatement`
   to `crates/smelt-logical/src/maintenance/emit.rs`, beside `emit_keyed_fold`, with a small
   dialect-aware null-safe-equality helper (no null-safe helper exists there today).
2. Add the pure disposition resolver to `crates/smelt-logical/src/contract/retain_departed.rs`:
   `DepartedKeyDisposition { Delete, Retain { tombstone: Option<String> } }` +
   `reconcile_disposition(Option<&RetainDeparted>)`. Re-export as the module's runtime seam.
3. In `execute_snapshot_reconcile` (`crates/smelt-runtime/src/cumulative.rs`), resolve the
   disposition from the model's `contract:` metadata; when `Delete`, assemble the merge statement
   and the delete statement into one `StatementGroup { transactional: true }` and execute it via
   `backend.execute_statement_group` (runtime assembles emitter output — it authors no SQL).
   Leave the first-run `CREATE TABLE AS` arm and the window-forward path untouched (departure is
   not observable there — `incremental_shapes.md` §"Departed keys and deletion").
4. Dispatch the retain-departed probe from `crates/smelt-runtime/src/contract_probes.rs`,
   following the `frozen_horizon`/`deferral` precedent: build it pure (empty set when the point is
   undeclared), pass the target table as `stored_table` and the compiled scan as a parenthesised
   subquery for `current_table`, execute at the pre-write site, record the retained count, and
   raise a violation on a non-zero unmarked-tombstone count.
5. Extend `statement_parity` with the family leg (test 6) and register the new emitter in its
   emitter-coverage imports.
6. Land the spec-delta edits and the stale in-code prose from §Spec delta.
7. Re-run the fixture-sensitive suites and repair any fixture that silently depended on retention
   (expect churn in `examples/web_analytics`-backed keyed tests; conformance should move *toward*
   its full-refresh oracle, not away).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-runtime --test departed_key_reconcile`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-cli --test example_web_analytics` and `cargo test -p smelt-logical --test walk_coverage`

## Commit message

`feat(incremental): delete departed keys at snapshot reconcile unless retain_departed is declared`
