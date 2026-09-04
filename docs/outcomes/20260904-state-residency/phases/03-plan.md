# Phase 3 plan — ledger statements under gate coverage; the delta-restricted write path recorded

## Objective

Close criterion 1's remaining surface: every DuckDB region-recompute write records its
`_smelt_ledger` reset in the same transaction — including the delta-restricted /
column-scoped-merge branch, which phase 2 left recording nothing — and put the ledger's
executed statements under standing gate coverage (statement-parity byte-parity against the
`smelt-state` builders, keyed-frontier upsert parity, and a transactional-rollback test).
Advances criteria 1 and 6 (the conformance gate stays green with the file ledger gone).

## Spec delta

None. Phase 1 already landed the normative statements in `docs/specs/state.md` §Surface/§Semantics
and `docs/specs/incremental_models.md` §"The reconciliation ledger"; this phase only makes the
implementation and its gates match them. No user-visible surface changes.

## Tests

Red-green list (all names new unless marked *extend*):

1. `crates/smelt-runtime/tests/statement_parity.rs::ledger_recompute_reset_statements_come_from_the_state_builder`
   — a real `execute_project` DeleteInsert run over `RecordingBackend`: the recorded raw SQL
   contains, per batch, exactly `generate_ledger_table_ddl(schema)` and the two
   `generate_ledger_recompute_reset_sqls(...)` strings byte-for-byte, and no ledger text appears
   inside any recorded maintenance `StatementGroup` (bookkeeping never leaks into the emitted write).
2. `crates/smelt-runtime/tests/statement_parity.rs::delta_restricted_recompute_records_the_ledger_reset`
   — the delta-restricted branch, driven through `execute_delete_insert_with_delta_restriction`,
   records the same byte-identical reset pair; red today (that path records nothing).
3. *extend* `crates/smelt-runtime/tests/statement_parity.rs::delta_restricted_recompute_statements_come_from_the_emitter`
   — the emitted write group is unchanged by the new bookkeeping arguments.
4. `crates/smelt-runtime/tests/keyed_frontier_bookkeeping.rs::merged_window_ledger_upsert_matches_the_state_builder`
   — the idempotent window-forward merge's recorded ledger SQL is byte-identical to
   `generate_ledger_upsert_sql(...)` (and its ensure DDL to `generate_ledger_table_ddl`), via a
   recording factory added to that file.
5. `crates/smelt-runtime/tests/statement_parity.rs::ledger_reset_rolls_back_with_a_failed_write`
   — `DuckDbBackend::execute_write_with_bookkeeping` with a valid ledger reset as `pre_write_sqls`
   and a deliberately invalid write `StatementGroup`: the call errors and `_smelt_ledger` holds no
   row for that region (proves "same transaction as the maintained write").
6. `crates/smelt-runtime/tests/statement_parity.rs::ledger_reset_is_skipped_on_a_non_duckdb_dialect`
   — a non-DuckDB dialect emits no ledger SQL and reports `state_structure_unavailable` (locks the
   documented gap phase 4 replaces with `MaintenanceStateDowngraded`).

## Tasks

1. Add `ensure_sqls: &[String]` + `pre_write_sqls: &[String]` parameters to
   `maintenance_driver::execute_delete_insert_with_delta_restriction`; route its terminal write
   through `Backend::execute_write_with_bookkeeping` when either is non-empty, else keep the
   existing `execute_statement_group` call (unchanged retry wrapping).
2. In `execute.rs`, hoist the phase-2 `ledger_reset_sqls` / ensure-DDL construction above the
   `use_delta_restricted_dispatch` fork so all three DuckDB DeleteInsert branches (model-edge
   restricted, external-sidecar restricted, plain) build the same reset from
   `generate_ledger_recompute_reset_sqls`, and pass it into both restricted call sites.
3. Write test 2, then 1, then 3 (red → green against tasks 1–2).
4. Add a `RecordingBackend`-style factory leg to `keyed_frontier_bookkeeping.rs` and write test 4.
5. Write tests 5 and 6.
6. Update the phase-2 comment block in `execute.rs` that names the plain branch as the only
   ledger-recording write path.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test keyed_frontier_bookkeeping --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test delta_restricted_recompute --test region_conditional_write --test web_analytics_session_delta_restriction --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -20`
- `rg -n 'reconciliation\.json' crates/` — no production reader/writer.

## Commit message

`test(state-residency): gate the engine-resident ledger statements and record the delta-restricted write path`
