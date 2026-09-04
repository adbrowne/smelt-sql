# Phase 2 summary — engine-resident reconciliation ledger on DuckDB

**Shipped:**
- `smelt-state/src/ddl_duckdb.rs`: `generate_ledger_recompute_reset_sqls` — the region-recompute
  DELETE (half-open intersection on `(model_name, grp)`) + INSERT pair, plus two unit tests.
- `smelt-backend/src/lib.rs`: `Backend::execute_model_incremental_with_bookkeeping` — the
  `(Table, Incremental{DeleteInsert})` arm on an existing table now builds the delete+insert
  group and routes it through `execute_write_with_bookkeeping` (DuckDB's real transactional
  override applies for free, no DuckDB-crate change needed); `execute_model_incremental` is now
  a thin call with empty `ensure_sqls`/`pre_write_sqls`, so every other caller is unaffected.
- `smelt-runtime/src/execute.rs`: the DuckDB DeleteInsert batch write builds the ledger `ensure`
  DDL + this batch's `[partition.start, partition.end)` reset (group `{*}`, input `self`, delta
  id = region end) and calls the new bookkeeping method; a non-DuckDB dialect reports the skip
  via `reporter.state_structure_unavailable` (same pattern `maintenance_driver`'s idempotent
  merge-ledger arm already uses) instead of a silent file fallback. The post-batch-loop
  `load_reconciliation_store`/`save_reconciliation_store` block and its `Processed`/`Region`
  imports are deleted.
- `smelt-state/src/file_store.rs`: `reconciliation_path`, `load_reconciliation_store`,
  `save_reconciliation_store` deleted; `reconciliation.json` dropped from the legacy-migration
  list (a legacy root-level file is now left in place, asserted by the updated
  `legacy_root_state_migrates_to_first_run_target` test).
- `smelt-state/src/reconciliation.rs`: `ReconciliationStore` deleted (its only consumer was the
  file store); `ReconciliationLedger`/`LedgerEntry`/`LedgerRecord`/`Region`/`Grade`/`Processed`
  kept — this crate's own tests still exercise them as the algebra's pure reference model; module
  doc gained a paragraph naming the SQL builders as the real, production-executed form.
- Test rewrites onto `_smelt_ledger` queries: `smelt-state/tests/reconciliation.rs`'s
  `fold_then_recompute_schedule_over_real_duckdb_model_matches_full_refresh` (leg 2) and
  `smelt-cli/tests/maintenance_conformance/probes.rs`'s
  `persisted_reconciliation_store_reflects_recompute_reset` → `engine_ledger_reflects_recompute_reset`;
  the file-store-only `reconciliation_store_roundtrips_through_file_store` test deleted (its API
  no longer exists).
- `smelt-state/CLAUDE.md`: layout list drops `reconciliation.json`; new gotcha bullet states the
  ledger is engine-resident.

**Decisions:**
- Scoped exactly to the plan's task list: only the plain (non-delta-restricted) DuckDB
  DeleteInsert branch in `execute.rs` gained the bookkeeping call. The delta-restricted/column-scoped-merge
  branch (`crate::maintenance_driver::execute_delete_insert_with_delta_restriction`, used when
  `use_delta_restricted_dispatch`) does **not** write a reconciliation-reset row — before this
  phase it relied on the now-deleted whole-run-window post-loop block for that; after this phase
  it writes none at all. Flagging as a gap below rather than silently expanding scope.
- Kept ledger DDL/DML builders in `smelt-state` (not `smelt-logical`), per the phase-1→phase-2
  decision-log correction: `smelt-logical`'s maintenance-plan-purity invariant explicitly excludes
  "ledger DDL/DML in `smelt-state`" as bookkeeping, and the existing MP12 builders already live
  there.
- To keep `retry_statement_group`'s closure a single concrete `Future` type,
  `execute_model_incremental` always routes through `execute_model_incremental_with_bookkeeping`
  (empty slices when there's nothing to record) rather than branching between the two methods per
  dialect inside the closure.

**For the next planner:**
- **Gap surfaced, not fixed**: the delta-restricted/column-scoped-merge write path
  (`execute_delete_insert_with_delta_restriction`) has no reconciliation-ledger reset at all now.
  Phase 3 (statement-parity/keyed-frontier coverage) or a dedicated follow-up should decide
  whether that path needs the same `_smelt_ledger` reset wired in, or whether its own existing
  MP12 fold-ledger interaction (via `fold_ledger_delta`/`Grade`) already subsumes the concern —
  this phase did not trace that far.
- Phase 4's `MaintenanceStateDowngraded` downgrade is the intended replacement for the
  `state_structure_unavailable` skip this phase adds on non-DuckDB dialects for the reset —
  matches the outcome's own plan, not a new discovery.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  `cargo test`, example_diagnostics).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 4 + 33 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 75 passed (generative conformance
  gate green with the file ledger gone).
- `rg -n 'reconciliation\.json' crates/` — hits only in comments/doc-prose and the
  legacy-migration test fixture string; no production reader/writer remains.
