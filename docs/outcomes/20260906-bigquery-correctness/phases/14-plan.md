# Phase 14 — Additive never-fold-twice on BigQuery without an enforced `PRIMARY KEY`

**Outcome:** `docs/outcomes/20260906-bigquery-correctness/outcome.md`
**Row:** 14 — "Additive never-fold-twice on BigQuery without an enforced `PRIMARY KEY`:
re-express the constraint-violation refusal transactionally, red-green on a repeat-fold test"
**Criteria served:** 8, 9, 3, 7
**Spec anchors:** `docs/specs/incremental_models.md` §Constraints "Never fold a delta already
reflected in the state", §"The frontier record (reconciliation ledger)"; `docs/specs/state.md`
§"Which dialects realise which structure", §"The degradation contract"

This is the outcome's own words for this row: *"the load-bearing phase: get it wrong and an
additive fold double-counts."* Treat correctness here as the deliverable and the row flip as a
consequence, not the other way round.

## Where phase 13 left it

Commit `183744c1b`. Already built, and **not** to be rebuilt:

- Every ledger builder in GoogleSQL (`crates/smelt-state/src/ddl_bigquery.rs`), including
  `generate_ledger_insert_sql`, `generate_ledger_exists_sql` and
  `generate_ledger_recompute_reset_sqls` — the reconciliation-ledger half is already written
  and dispatched even though its row is off.
- The one dialect dispatch, `crates/smelt-state/src/ledger.rs` (`ledger_table_ddl`,
  `ledger_insert_sql`, `ledger_upsert_sql`, `ledger_exists_sql`,
  `ledger_recompute_reset_sqls`), exhaustive over `SqlDialect`, Spark erroring by name.
- `escape_string_literal` — phase 13 found DuckDB's `''`-doubling is not GoogleSQL-portable.
  Use it; do not reintroduce a second escaper.
- BigQuery's `execute_write_with_bookkeeping` override and the pure
  `sql::write_with_bookkeeping_plan` it is built from — `ensure_sqls` first and outside, then
  one `BEGIN … BEGIN TRANSACTION; … COMMIT TRANSACTION; EXCEPTION WHEN ERROR THEN ROLLBACK
  TRANSACTION; RAISE …; END` script. The explicit rollback is deliberate: BigQuery does not
  unwind a script's transaction on its own.
- `crates/smelt-runtime/src/maintenance_driver/ledger.rs::realises_merge_ledger`, derived from
  `realisable_state_structures` rather than compared against `SqlDialect::DuckDB` — the pattern
  this phase should follow for its own gate.

`realisable_state_structures(BigQuery)` is `vec![MergeLedger]`. Three `STATE-GUARD` sites
remain: `driver.rs:485` (`ReconciliationLedger`, this phase) and `succession/execute.rs:97,312`
(`TombstoneLedger`, phase 15).

## The actual problem

On DuckDB the never-fold-twice guarantee **is** a storage constraint. `fold_ledger_delta`'s
DuckDB override (`crates/smelt-backend-duckdb/src/lib.rs:726+`) runs `insert_sql` inside a
`duckdb::Transaction`; a repeat delta violates the ledger's `PRIMARY KEY`, the error is
recognised by `is_constraint_violation`, and it surfaces as `BackendError::already_reflected`
with `action_sql` never executed — the transaction's `DropBehavior::Rollback` doing the undo.
No check-then-act window exists because the check *is* the write.

BigQuery's `PRIMARY KEY` is `NOT ENFORCED`. It raises nothing. So the insert silently succeeds
twice and an additive fold double-counts — which is why `driver.rs:485` currently refuses
outright rather than degrading.

The trait's default `fold_ledger_delta` is **not** an acceptable answer here. It is documented
as a best-effort non-atomic fallback: `exists_sql`, then `insert_sql`, then `action_sql` as
three separate statements. Two concurrent runs can both read "absent" and both fold. Shipping
that as BigQuery's realisation would satisfy the census and reintroduce the exact defect the
row exists to prevent.

## The shape to build (and what to verify rather than assume)

Re-express the refusal as a **conditional insert whose zero-row outcome aborts the script**,
inside the same transaction as the action:

```
BEGIN
BEGIN TRANSACTION;
INSERT INTO `<schema>._smelt_ledger` (model_name, grp, input_name, delta_id, region_start, region_end)
SELECT <literals> WHERE NOT EXISTS (
  SELECT 1 FROM `<schema>._smelt_ledger`
  WHERE model_name = <lit> AND grp = <lit> AND input_name = <lit> AND delta_id = <lit>);
IF @@row_count = 0 THEN
  ROLLBACK TRANSACTION;
  RAISE USING MESSAGE = '<sentinel>: …';
END IF;
<action_sql>;
COMMIT TRANSACTION;
EXCEPTION WHEN ERROR THEN … END;
```

Decisions the implementer must make explicitly, each stated in code comments and the summary:

1. **The sentinel.** The Rust side has to tell "already reflected" apart from any other script
   failure and map it to `BackendError::already_reflected`, exactly as `is_constraint_violation`
   does for DuckDB. Pick a sentinel string that cannot collide with user data, put the matcher
   next to the builder that emits it, and test both halves — a recogniser that can drift from
   its emitter is the bug this phase is least able to afford.
2. **Isolation.** The soundness argument is that BigQuery multi-statement transactions get
   snapshot isolation with write-conflict detection, so two concurrent folds of the same delta
   cannot both commit — one aborts. **Verify this against the BigQuery docs before relying on
   it** and write the argument into the doc comment. If the guarantee turns out weaker than
   the refusal needs, say so and refuse rather than shipping a race; an honest continued
   refusal here is a better outcome than a false realisation, and the outcome's own framing
   supports that.
3. **DDL in the transaction — phase 13 flagged this and here it is load-bearing.** BigQuery
   does not permit DDL inside a transaction, and `action_sql` for a fold is documented as
   "a `CREATE TABLE … AS` or `MERGE INTO` statement". A `CREATE TABLE … AS` action cannot go
   in the script. Decide and implement one of: (a) refuse the DDL-shaped action on BigQuery
   with a clear `UnsupportedOnBackend`-style message naming the construct, or (b) establish
   that the additive-fold path never produces a DDL action (prove it from the call site, don't
   assert it). Do **not** leave it to be discovered live — that would repeat the class of
   defect phase 12's live run found. Whichever way it goes, state it in the spec.
4. **`exists_sql`'s fate.** If the conditional insert makes the separate existence check dead
   on BigQuery, say so rather than passing it and ignoring it silently.

## Flipping the row

`realisable_state_structures(SqlDialect::BigQuery)` → `vec![MergeLedger, ReconciliationLedger]`,
and the census then forces `driver.rs:485`'s guard to go. Follow phase 13's precedent: prefer
retiring the guard in favour of a **derived predicate** in
`crates/smelt-runtime/src/maintenance_driver/ledger.rs` (a `realises_reconciliation_ledger`
sibling to `realises_merge_ledger`) over re-annotating a `!= SqlDialect::DuckDB` comparison —
the census module doc names that as the preferred fix. If the refusal must remain for a
narrower reason (e.g. decision 3 lands on "refuse DDL actions"), that is a *different*
condition and must not be spelled as a dialect comparison.

`Technique::KeyedFold` becomes admissible on BigQuery as a result. Expect fallout in the
availability tests and in `examples/github_activity`'s recorded diagnostics — phase 13 saw
exactly this when a `ColumnScopedMerge` downgrade stopped firing, and the *absence* of the
downgrade became the assertion. Check `crates/smelt-cli/tests/example_diagnostics*` and
`crates/smelt-runtime/src/execute/project/mod.rs:3611`'s `ReconciliationLedger` reference.

**Correct pre-existing tests that encode the old claim; never delete them.**

## Tests (red-green, written before the fix)

1. **The repeat-fold test the row names.** Fold the same `(model, grp, input, delta_id)` twice
   against a recorded-statement fake backend driven by the real BigQuery SQL builders; the
   second must produce `BackendError::already_reflected` and the action must not appear in the
   second call's statement log. This is the phase's headline test — make it read like one.
2. **Emitter/recogniser pairing.** The sentinel the builder emits is the sentinel the Rust
   matcher recognises, asserted in one test so they cannot drift.
3. **Atomicity shape.** The emitted script contains the conditional insert, the `IF @@row_count
   = 0` abort and `action_sql` inside exactly one `BEGIN TRANSACTION … COMMIT TRANSACTION`,
   with the ensure DDL outside it — asserted against the pure plan builder, the way phase 13's
   `write_with_bookkeeping_plan` tests are written.
4. **No `ON CONFLICT`, no `"`-quoted identifier, no `''` escaping** in any BigQuery ledger text
   (extend `crates/smelt-state/tests/ledger_dialect.rs` rather than starting a new file).
5. **Census** green with the `ReconciliationLedger` guard retired.
6. **Availability:** a `KeyedFold` cell on BigQuery records **no** `ReconciliationLedger`
   downgrade; Spark still does.
7. Whatever decision 3 lands on gets its own test — a refusal test if you refuse, a
   proof-from-the-call-site test if you establish the action is never DDL.

**Do not reach a live BigQuery warehouse.** Offline proof plus `cargo check -p smelt-cli
--features bigquery`. Phase 16 owns live verification, and it now inherits three things to
check there: the DDL-in-transaction question, that the `MERGE` upsert really no-ops on a repeat
window, and that this phase's repeat fold really refuses on the real engine.

## Spec delta (spec-first)

- `docs/specs/state.md` §"Which dialects realise which structure": BigQuery's
  reconciliation-ledger row moves to realised, with the mechanism named — the refusal comes
  from a conditional insert inside a transaction, not from a key constraint — and the isolation
  argument from decision 2 stated as the property it depends on.
- `docs/specs/incremental_models.md` §Constraints "Never fold a delta already reflected in the
  state": the guarantee becomes dialect-plural. It is currently written as a storage-constraint
  fact; it must become "the ledger refuses a repeat", with the two realisations named
  underneath. The reopening entry already anticipated this ("never-fold-twice becomes
  dialect-plural").
- Anything decision 3 constrains (a refused action shape) belongs in
  `docs/specs/multi_backend.md` alongside the other clause-level dialect refusals.

## Gates

```
bash .claude/scripts/verify-phase.sh
cargo test -p smelt-runtime --test state_guard_census
cargo test -p smelt-runtime --test availability_seam
cargo test -p smelt-logical --test maintenance_availability
cargo test -p smelt-logical --test maintenance_dialect_blindness
cargo test -p smelt-state --test ledger_dialect
cargo check -p smelt-cli --features bigquery
bash .claude/scripts/large-file-check.sh
```

`ddl_bigquery.rs` reached 967 lines in phase 13. If this phase pushes it over its cap, split it
(`ddl_bigquery/{mod.rs,ledger.rs}`) rather than raising the ratchet.

## Deliverables

- The code + tests above.
- `docs/outcomes/20260906-bigquery-correctness/phases/14-summary.md`.
- A decision-log entry at the **top** of the outcome's `## Decision log`, in the voice of the
  existing entries: dense, file:line specific, explicit about what is *not* proven offline.
- Row 14 flipped to `done`.
- One commit on `bigquery-prod` with the session's trailers.
