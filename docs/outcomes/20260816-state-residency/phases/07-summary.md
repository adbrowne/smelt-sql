# Phase 7 summary — fuse the frontier reset into the region recompute's write transaction

## Shipped

- `maintenance_driver::execute_region_recompute_with_frontier_reset` (`crates/smelt-runtime/src/maintenance_driver.rs`):
  builds the frontier's ensure/reset-delete/insert SQL from `smelt_state::ddl_duckdb` and hands
  it, together with the caller's already-built `StatementGroup`, to
  `Backend::execute_write_and_reset_frontier` under `retry_backend_call` — one call, one
  transaction, no new statement text authored.
- `execute.rs`'s ordinary DuckDB `DeleteInsert` batch branch now builds `emit_delete_insert`'s
  group once, reports it (as before), then — when the target already exists — hands that SAME
  group to the new fused helper instead of calling `execute_model_incremental` +
  a separate after-loop frontier write. An absent target (bootstrap `CREATE TABLE AS`) still
  falls through to `execute_model_incremental` unchanged.
- A `fused_batch_writes` counter gates the after-the-loop whole-range frontier write: it now
  runs only when at least one batch in the run was *not* fused (bootstrap, delta-restricted
  recompute, column-scoped merge / in-place update).
- Spec delta: `docs/specs/incremental_models.md` §"The frontier record (reconciliation ledger)"
  now states the record is written **per recomputed batch region**, and a new §Known Divergences
  bullet names the three write paths that still aren't fused (bootstrap, delta-restricted
  recompute, column-scoped-merge / in-place-update).
- Three new tests in `crates/smelt-runtime/tests/frontier_residency.rs`:
  `frontier_record_is_written_per_batch_region` (3-day/1-day-batch recompute over an existing
  target → 3 distinct `_smelt_frontier` rows, not one whole-range row),
  `bootstrap_run_still_records_the_frontier` (unfused after-loop path still fires for a
  bootstrap run), `failed_batch_write_records_no_frontier_row` (a `FailingFrontierBackend`
  wrapper fails the fused transaction on the second batch — that batch's data write AND
  frontier row both roll back atomically; the third batch never runs; the first batch's fused
  write and row stand).

## Decisions

- Injected a synthetic backend failure (`FailingFrontierBackend`, mirrors `retry.rs`'s
  `FailingBackend` delegating-wrapper pattern) rather than a SQL-level `CAST` error keyed on a
  batch's date, to test atomicity: the outer time-filter wrap around the model's inner SELECT
  means a `CAST` error embedded in the model body would fire on every batch's inner-query
  evaluation regardless of that batch's own WHERE clamp (no source-level predicate pushdown for
  a raw-VALUES fixture model), so it could not isolate a single batch's failure deterministically.
  The wrapper gives an exact, deterministic failure point without depending on query-optimizer
  behavior.
- `table` was dropped from the helper's planned signature — the `StatementGroup` the caller
  passes already carries the fully-qualified table name (`emit_delete_insert` embeds it), so a
  separate `table` parameter would go unused and trip `unused_variables`.

## For the next planner

- Row 8 (state-deletion conformance leg) can now assert per-batch frontier residency across
  `.smelt/` deletion / fresh-clone scenarios for the fused DuckDB `DeleteInsert` path
  specifically, in addition to the whole-row keyed-fold and idempotent-graded cases the row
  already names.
- The three residual unfused paths (bootstrap, delta-restricted recompute, column-scoped
  merge/in-place-update) are now honestly named in §Known Divergences rather than silently
  covered by a bullet that read as fully closed — fusing them was scoped out of this phase
  (see the outcome's phase-7 decision log) and stays available as later work if a real
  workload needs it.

## Gates

- `bash .claude/scripts/verify-phase.sh` (full) — ALL GREEN.
- `cargo test -p smelt-runtime --test frontier_residency --test statement_parity --test execute_parity --test state_posture` — 40 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 70 passed.
- `cargo test -p smelt-state --test reconciliation` — 5 passed.
- `cargo test -p smelt-backend-duckdb --lib` — 31 passed.
- Timeless-oracle grep on the spec edits — clean.
- `.claude/hardening-baseline.txt`: `smelt-runtime expect` 10→11 (the new
  `ordinary_group.expect("built above whenever can_fuse_frontier_reset")`, same infallible
  pattern as the existing `column_scoped_cell` `.expect()` a few lines above it in `execute.rs`).
