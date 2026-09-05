# Phase 2 plan — engine-resident reconciliation ledger on DuckDB

**Outcome:** `docs/outcomes/20260904-state-residency/outcome.md` (criterion 1)

## Objective

Move the last file-resident reconciliation-ledger write — the region-recompute reset the batch
loop performs in `crates/smelt-runtime/src/execute.rs` — onto the existing warehouse-resident
`_smelt_ledger` table, executed in the same backend transaction as the batch's own DELETE+INSERT
write. Delete `.smelt/reconciliation.json` and every `smelt-state` API that persists it. Advances
criterion 1; leaves the parity/conformance widening to phase 3.

## Context the implementer must not re-derive

- The **fold** half of criterion 1 is already engine-resident: `_smelt_ledger`
  (`smelt-state/src/ddl_duckdb.rs` §"Warehouse-resident per-delta reconciliation ledger (MP12)")
  with the `PRIMARY KEY (model_name, grp, input_name, delta_id)` as the never-fold-twice key,
  driven through `Backend::fold_ledger_delta` (DuckDB overrides it transactionally). Nothing in
  that path changes.
- The **only** remaining production writer of `.smelt/reconciliation.json` is the post-batch-loop
  block in `execute.rs` (~L4496-4530): a whole-row `{*}` `recompute_reset` recording
  `Frontier{"self": end_str}` for the run window. Everything else touching
  `load/save_reconciliation_store` is a test.
- `Backend::execute_write_with_bookkeeping(ensure_sqls, pre_write_sqls, write_group)` is the
  existing seam for "record something in the same transaction as a write"; DuckDB overrides it
  with a real `duckdb::Transaction`.

## Spec delta

None. Phase 1 already made `docs/specs/state.md` §"The residency rule" and `run_state.md` the
normative statement; this phase makes the implementation match a spec that is already correct.

## Tests (red first)

1. `smelt-state/src/ddl_duckdb.rs::ledger_recompute_reset_deletes_intersecting_and_records_read`
   — the new builder emits a `DELETE` whose predicate is half-open intersection
   (`region_start < :end AND region_end > :start`) scoped to `(model_name, grp)`, followed by the
   `INSERT` recording the input state read.
2. `smelt-state/src/ddl_duckdb.rs::ledger_recompute_reset_escapes_literals` — model/group/input
   values with `'` are escaped.
3. `smelt-backend/tests/…` (or an inline unit test) `incremental_with_bookkeeping_runs_reset_in_write_transaction`
   — the new trait method routes the existing-table DeleteInsert case through
   `execute_write_with_bookkeeping` with the bookkeeping statements ahead of the write group.
4. `smelt-runtime/tests/reconciliation_residency.rs::region_recompute_records_reset_in_engine_ledger`
   — after an incremental run over two windows, `_smelt_ledger` holds one `{*}` row per written
   region for the model, with `input_name = 'self'` and `delta_id` = the region end.
5. `…::second_run_over_same_region_replaces_not_accumulates` — re-running the same window leaves
   exactly one `{*}` row for that region (recompute-reset semantics, not append).
6. `…::failed_write_leaves_no_ledger_reset_row` — a batch write that fails rolls back the reset
   row (DuckDB transactional leg).
7. `…::run_writes_no_reconciliation_json` — no `.smelt/**/reconciliation.json` exists after a run.
8. Rewrite `smelt-cli/tests/maintenance_conformance/probes.rs::persisted_reconciliation_store_reflects_recompute_reset`
   → `engine_ledger_reflects_recompute_reset`: assert the same two regions against
   `_smelt_ledger` via a DuckDB query instead of the file store.

## Tasks

1. `smelt-state/src/ddl_duckdb.rs`: add `generate_ledger_recompute_reset_sqls(schema, model,
   group, region_start, region_end, input, delta_id) -> Vec<String>` (DELETE-intersecting +
   INSERT) beside the existing MP12 builders, with the doc comment stating it is the
   region-recompute half of the ledger and why it lives in `smelt-state` (bookkeeping exclusion,
   CLAUDE.md maintenance-plan purity rule, same precedent as `generate_ledger_insert_sql`).
2. `smelt-backend/src/lib.rs`: add `execute_model_incremental_with_bookkeeping(…, ensure_sqls,
   pre_write_sqls)`. Default impl: for `(Table, Incremental{DeleteInsert})` on an existing table,
   build the delete+insert group via the shared emitter and call
   `execute_write_with_bookkeeping`; every other arm runs `ensure_sqls`/`pre_write_sqls` then
   delegates to today's logic. Make `execute_model_incremental` a thin call with empty slices so
   there is still exactly one write path. No DuckDB override needed.
3. `execute.rs` batch loop: for the DuckDB DeleteInsert branch, build the ledger `ensure` DDL plus
   the reset statements for **this batch's** `[partition.start, partition.end)` region (group
   `{*}`, input `self`, delta id = region end) and call the new method. On a dialect with no
   ledger builder, call the plain method unchanged and leave a comment naming phase 4
   (`MaintenanceStateDowngraded`) as the owner of that gap — no silent file fallback.
4. `execute.rs`: delete the post-loop `load_reconciliation_store`/`save_reconciliation_store`
   block and the now-unused `Processed`/`Region` imports.
5. `smelt-state/src/file_store.rs`: delete `reconciliation_path`, `load_reconciliation_store`,
   `save_reconciliation_store`, and `"reconciliation.json"` from the legacy-migration list; update
   `legacy_root_state_migrates_to_first_run_target` to assert a legacy root-level
   `reconciliation.json` is **left in place** (per `run_state.md`'s legacy-migration clause).
6. `smelt-state/src/reconciliation.rs`: delete `ReconciliationStore` and any type that becomes
   unused once the file store is gone (`cargo clippy` dead-code is the oracle); keep `Region`,
   `Grade`, `Processed`, which runtime and the testkits still consume. Trim the module doc to the
   engine-resident story and update `smelt-state/tests/reconciliation.rs` accordingly.
7. Rewrite the conformance probe test (test 8) and run the gates.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -20`
- `rg -n 'reconciliation\.json' crates/ docs/specs/` — production hits gone; remaining hits only in
  `run_state.md`/`state.md` divergence-class prose.

## Commit message

`feat(state-residency): move the region-recompute ledger reset into the engine-resident _smelt_ledger`
