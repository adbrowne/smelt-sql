# Phase 7 plan — fuse the frontier reset into the region recompute's write transaction

## Objective

Make the code match what `incremental_models.md` §"The frontier record (reconciliation ledger)"
and `state.md` §"The residency rule" already claim: a region recompute's frontier reset commits in
the **same** backend transaction as that recompute's own write, not in a separate transaction
after the model's batch loop has already committed. Advances criterion 2 (the ledger is engine
resident *and* transactional with the fold) and criterion 6 (no spec-vs-code drift).

## Ground truth (verified while planning)

- The write happens per batch (`execute_model_incremental` → `delete_and_insert_transactional`,
  `crates/smelt-runtime/src/execute.rs:3542`); the frontier reset happens **once after** the batch
  loop over the whole `[start,end)` range with an empty `write_group` (`execute.rs:3732`).
- `Backend::execute_write_and_reset_frontier`'s DuckDB override is already a real single
  transaction over `write_group + reset delete + insert`, and its atomicity is already covered by
  `smelt-backend-duckdb`'s `failed_write_leaves_frontier_untouched`. The gap is purely the **call
  site**: it passes no write.
- The ordinary DuckDB `DeleteInsert` branch already builds the exact executed group via
  `emit_delete_insert` for reporting (`execute.rs:3505`) — that group is what must be handed to the
  hook, keeping statement-emission single ownership intact.
- Three sibling write paths in the same loop are **not** in scope to fuse: the bootstrap
  `CREATE TABLE AS` first materialization, the delta-restricted recompute
  (`execute_delete_insert_with_delta_restriction`), and column-scoped MERGE / in-place update.
  They keep today's after-the-loop record.

## Spec delta (spec-first, made by the implement step)

- `docs/specs/incremental_models.md` §"The frontier record (reconciliation ledger)": one sentence —
  the reset is written per **recomputed batch region** (batches partition the run's window; the
  region-intersecting delete keeps the record collapsible), inside that batch's write transaction.
- `docs/specs/incremental_models.md` §Known Divergences: narrow the residency bullet to name the
  residual paths whose frontier record is still written after the model completes rather than with
  its write — the first-run `CREATE TABLE AS` materialization, the delta-restricted recompute, and
  the column-scoped-merge / in-place-update techniques. Honest narrowing, not a new gap.

## Tests (red-green)

1. `frontier_residency::frontier_record_is_written_per_batch_region` — an existing target
   recomputed over a 3-day window with 1-day batches yields three `_smelt_frontier` rows with
   `region_start` per batch (red today: one whole-range row).
2. `frontier_residency::failed_batch_write_records_no_frontier_row` — a model whose SQL fails on
   the second batch (e.g. a runtime `CAST` error keyed on that batch's date): the run errors, the
   target holds only batch 1's rows, and `_smelt_frontier` holds only batch 1's region — no record
   for a write that never committed.
3. `frontier_residency::bootstrap_run_still_records_the_frontier` — the first run (table created by
   `CREATE TABLE AS`, no fused batch) still records the whole-range frontier entry via the retained
   after-the-loop path.
4. Existing `maintenance_conformance::persisted_reconciliation_store_reflects_recompute_reset` must
   stay green unchanged (its runs are single-batch, so per-batch == whole-range).

## Tasks

1. Write tests 1–3 red in `crates/smelt-runtime/tests/frontier_residency.rs`.
2. Make the spec delta above.
3. Add `maintenance_driver::execute_region_recompute_with_frontier_reset(backend, schema, table,
   &group, model_name, region_start, region_end, retry)`: builds ensure/reset-delete/insert SQL from
   `smelt_state::ddl_duckdb`, calls `Backend::execute_write_and_reset_frontier` under
   `retry_backend_call`. Backends execute; the write text stays the emitter's group.
4. In `execute.rs`'s ordinary batch branch, when `dialect == DuckDB && resolved_strategy ==
   DeleteInsert && backend.table_exists(schema, db_name)`, execute via the new helper with the group
   already built for the report, then read `row_count` via `backend.get_row_count` (byte-identical to
   what `execute_model_incremental` returns today). Otherwise keep `execute_model_incremental`.
5. Count fused batches; run the existing after-the-loop whole-range frontier record **only** when
   `fused < inc_plan.batches.len()` (so bootstrap/unfused runs keep their record and fully fused runs
   don't double-write a coarser row).
6. Rewrite the `execute.rs:3703` comment block: it currently documents `write_group` being empty as
   the accepted state; it must document the fused path plus the named residual paths.

## Verification

- `bash .claude/scripts/verify-phase.sh` (full)
- `cargo test -p smelt-runtime --test frontier_residency --test statement_parity --test
  execute_parity --test state_posture`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-state --test reconciliation`
- `cargo test -p smelt-backend-duckdb --lib`
- Timeless-oracle check on the two spec edits (no phase vocabulary).

## Commit message

`feat(state): commit the frontier reset in the region recompute's own write transaction`
