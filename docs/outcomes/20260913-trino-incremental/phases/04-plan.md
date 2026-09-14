# Phase 4 plan — the emulated delete-and-insert window covers exactly what it writes

## Objective

Trino has no `INSERT OVERWRITE`, so the delete-and-insert window is emulated as a scoped
`DELETE` + `INSERT` pair. Prove the emulation's central property — the `DELETE` covers precisely
the range the `INSERT` writes, no wider (data loss) and no narrower (duplicates) — directly at the
statement level off a real `execute_project` run, and behaviourally under out-of-order and
repeated application. Advances criteria 2 (`DeleteInsert` executes), 4 (exact coverage) and 5
(statement_parity's Trino leg gains a second family).

## Spec delta (the implement step makes these edits first)

1. `docs/specs/incremental_shapes.md` §"First-run and backfill" — the sentence "Each chunk's
   DELETE+INSERT is one transaction" is false on a backend whose writes are autocommit-only.
   Qualify it: the pair is one transaction **where the backend realises a multi-statement
   transaction** (DuckDB); on Trino/Iceberg and Spark it executes sequentially, so an `INSERT`
   failure leaves the chunk's window empty rather than rolling back. State the recovery property
   that makes that safe: because the `DELETE` covers exactly the window the `INSERT` writes and
   nothing outside it, re-running the same window restores the chunk — the same idempotence the
   paragraph already relies on for resumption.
2. `docs/specs/multi_backend.md` §"Incremental & schema evolution per backend" — the `DeleteInsert`
   row of the per-`Technique` table gains the non-atomicity note and a pointer to the
   `incremental_shapes.md` sentence above, so the emulation's failure mode is stated where a
   reader checks what Trino supports. No new capability flag and no new diagnostic code (the
   section's own no-new-code rule).

## Tests

Pure / no live tier:
1. `smelt-logical` `recompute.rs` — `delete_leg_predicate_is_exactly_the_insert_window`: for a
   region `[a, b)`, `emit_delete_insert`'s `DELETE` predicate is byte-identical to
   `region.predicate(None, partition_col)` — the emitter neither widens nor narrows, and adds no
   second filter to the `INSERT` (which carries the caller's already-clamped body verbatim).
2. `smelt-backend-trino` unit — `insert_overwrite_is_emulated_not_refused`: `TrinoBackend`'s
   `insert_overwrite` no longer returns `BackendError::unsupported` (asserted on the error-free
   construction path / via a recording double, no coordinator needed).

Live tier (`SMELT_TRINO_URL`; skip with an explicit `Skipping …` line, never silently):
3. `crates/smelt-runtime/tests/statement_parity/trino.rs::delete_insert_parity_on_trino` — the
   `DeleteInsert` group executed during a real `execute_project` Trino run, captured by
   `RecordingBackend`, is byte-identical to a direct `emit_delete_insert` call with the batch's own
   `Region`; **and** the `DELETE`'s two literals are exactly the batch's `TimeRange` bounds, which
   are the same two literals the `INSERT` body's injected output clamp carries. This is
   criterion 4's "asserted directly".
4. `crates/smelt-cli/tests/trino_incremental_families.rs::
   delete_insert_window_replaces_only_its_own_rows_on_trino` — run window `[1, 3)`, mutate the
   source rows of batch 2 only, re-run window `[2, 3)`: batch-1 rows are byte-identical to before
   (not deleted — no wider), batch-2 rows reflect the mutation exactly once (not duplicated — no
   narrower), and no row outside `[1, 3)` appeared.
5. `…::delete_insert_windows_applied_out_of_order_match_full_refresh_on_trino` — apply `[3, 5)`
   then `[1, 3)`; the maintained table is multiset-equal to a `--full-refresh` oracle in a
   separate schema.
6. `…::delete_insert_repeated_window_is_idempotent_on_trino` — apply `[1, 3)` twice with no source
   change; the table equals the single-application result exactly (no duplicate rows, no loss).

## Tasks

1. Make the two spec edits above.
2. `TrinoBackend::insert_overwrite`: delegate to `delete_and_insert_transactional` (BigQuery's
   `lib.rs` precedent — lower, don't reject), deleting the by-name refusal and its "phase 4's
   subject" module-doc line.
3. `TrinoBackend::delete_and_insert_transactional`: override the trait default so the emitted text
   targets `self.qualified_name(schema, name)` (`"cat"."sch"."tbl"`), exactly as Spark's and
   BigQuery's overrides do; the text still comes from `emit_delete_insert` — author nothing here.
4. `TrinoBackend::delete_partitions`: it is on no runtime path (`rg` finds no caller outside a test
   double), so leave it refusing, but rewrite the message to say that rather than pointing at this
   outcome — a stale "lands in phase 4" pointer after phase 4 is a lie. Record the choice in the
   summary so phase 7's structural leg does not have to rediscover it.
5. Add tests 1-2, then 3-6. Each live test takes its own `trino_schema(...)`-derived unique schema
   and drops it unconditionally (3e's isolation rule); the oracle run in test 5 gets a second one.
6. Update `trino_incremental_families.rs`'s module doc: the delete-and-insert window family is
   proved end-to-end, with the three behavioural properties named.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, workspace
  tests, example_diagnostics).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` with no live tier —
  the Trino leg must skip cleanly, not fail.
- `cargo test -p smelt-cli --test trino_ci_wiring` — the no-skip and shared-schema-helper gates.
- Live tier (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`; teardown after):
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1`,
  `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1`,
  `cargo test -p smelt-backend-trino --test backend_live -- --test-threads=1`.
- If the coordinator is unreachable, emit `<<PHASE_BLOCKED>>` — never report the live legs green.

## Commit message

`feat(trino): emulate the delete-and-insert window and prove its DELETE covers exactly the insert's write window`
