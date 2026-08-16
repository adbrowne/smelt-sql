# Phase 4 plan — the reconciliation ledger goes engine-resident

## Objective

Move the frontier record out of `.smelt/reconciliation.json` into a backend table written in
the same transaction as the write it records, and consume-then-remove any legacy JSON ledger on
first run. Advances criterion 2 (engine-resident ledger, never-fold-twice off `.smelt/`) and is
the precondition for criterion 5's state-deletion conformance leg.

**Ground truth discovered while planning** (the `state.md` Known Divergence overstates the gap):
the **additive** grading is *already* engine-resident — `_smelt_ledger`
(`smelt_state::ddl_duckdb::generate_ledger_table_ddl`) with its
`PRIMARY KEY (model_name, grp, input_name, delta_id)` *is* the never-fold-twice key, committed
with the fold action by `Backend::fold_ledger_delta` (DuckDB transactional override,
`crates/smelt-runtime/src/maintenance_driver.rs:400`). What still lives in
`.smelt/reconciliation.json` is the **idempotent/frontier** grading: the whole-row `{*}`
region-recompute reset written at `crates/smelt-runtime/src/execute.rs:3652`. Nothing in
production ever *reads* `.smelt/reconciliation.json` — it is write-only bookkeeping — so this
phase moves a write, not a decision.

## Spec delta (spec-first; the implement step makes these edits)

1. `docs/specs/run_state.md` §Layout — drop `reconciliation.json` from the layout block and from
   §Known Divergences-adjacent "Fixed layout" bullet; drop "the reconciliation ledger" from the
   §Locking sentence enumerating shared whole-store files serialized within a run. Keep it in the
   legacy-layout migration list (a pre-versioning `.smelt/` still moves it under `targets/<t>/`
   before it is imported).
2. `docs/specs/run_state.md` §"Relationship to the reconciliation ledger" — replace "today stored
   under `.smelt/reconciliation.json` for both storage gradings — a divergence from its normative
   residency" with the end state: both gradings are engine-resident, each write transactional with
   the write it records; plus one sentence on the legacy import (a `reconciliation.json` left by an
   older binary is imported into the engine tables on the first run that can reach a ledger-capable
   backend, then removed; on a backend with no ledger builder it is left untouched and the run says
   so).
3. `docs/specs/incremental_models.md` §"The frontier record (reconciliation ledger)" — one
   paragraph: the record is engine-resident and graded by algebra into two tables — additive delta
   identities in `_smelt_ledger`, idempotent frontier watermarks in `_smelt_frontier` — and a
   region recompute's reset (delete every intersecting `(region, group)` row, insert the read
   state) commits in the same backend transaction as the recompute's own `DELETE`+`INSERT`.
4. Known Divergences the move falsifies: `state.md` bullet 2 (rewrite to the residual gap — no
   ledger builder outside DuckDB, whose downgrade is phase 5 — or delete if nothing residual
   remains), and the matching `run_state.md` / `incremental_models.md` bullets. Phase 7 still
   sweeps the rest; do not leave a false bullet standing here.

## Tests (red-green, in this order)

- `smelt-state` `ddl_duckdb::frontier_table_ddl_is_idempotent_and_keys_region_group` — DDL is
  `CREATE TABLE IF NOT EXISTS <schema>._smelt_frontier` keyed `(model_name, grp, region_start)`.
- `smelt-state` `frontier_reset_sql_scopes_delete_to_intersecting_regions_of_the_same_group` —
  the reset `DELETE` predicate is `model_name = … AND grp = … AND region_start < 'end' AND 'start'
  < region_end`, so a non-intersecting region and a sibling group survive.
- `smelt-state` `tests/reconciliation.rs::engine_reset_matches_in_memory_recompute_reset` —
  executed against a real DuckDB: the emitted reset DML leaves the same `(region, group)` set as
  `ReconciliationLedger::recompute_reset` on the same inputs.
- `smelt-backend-duckdb` `write_and_reset_frontier_commits_together` /
  `failed_write_leaves_frontier_untouched` — the new `Backend` hook's transactionality both ways.
- `smelt-state` `file_store::take_legacy_reconciliation_store_returns_it_and_removes_the_file` /
  `…_returns_none_when_absent`.
- `smelt-runtime` `tests/frontier_residency.rs::incremental_run_records_the_frontier_in_the_engine`
  — after a real `execute_project` incremental run on DuckDB, `_smelt_frontier` holds the region
  row and `.smelt/targets/<t>/reconciliation.json` does not exist.
- `smelt-runtime` `tests/frontier_residency.rs::legacy_reconciliation_json_is_imported_then_removed`
  — a seeded legacy file (one `Frontier` and one `DeltaIdentities` record) lands in
  `_smelt_frontier` / `_smelt_ledger` respectively and the file is gone.
- `smelt-runtime` `tests/state_posture.rs` — replace `reconciliation_store_ignores_the_posture`
  (it asserts the JSON file *is* written under `stateless`) with
  `stateless_run_still_records_the_frontier_in_the_engine`.

## Tasks

1. Make the four spec edits above.
2. `smelt-state/src/ddl_duckdb.rs`: add `FRONTIER_TABLE_NAME = "_smelt_frontier"` plus
   `generate_frontier_table_ddl`, `generate_frontier_reset_delete_sql`,
   `generate_frontier_insert_sql` (columns `model_name, grp, input_name, delta_id, region_start,
   region_end`, matching `_smelt_ledger`'s shape, `PRIMARY KEY (model_name, grp, region_start)`).
   A separate table, not a `grade` column on `_smelt_ledger`: adding a column would need a
   warehouse-side migration of existing `_smelt_ledger` tables, and a shared `{*}` group would let
   a frontier reset's intersecting-region `DELETE` wipe additive delta-identity rows.
3. `smelt-backend`: add `Backend::execute_write_and_reset_frontier(ensure_sql, write_group,
   reset_delete_sql, insert_sql)` with a non-atomic default, modelled verbatim on
   `execute_write_and_refresh_fingerprint_sidecar` (`crates/smelt-backend/src/lib.rs:531`);
   `smelt-backend-duckdb` overrides it with a single-transaction implementation.
4. `smelt-state/src/file_store.rs`: delete `save_reconciliation_store`, demote
   `load_reconciliation_store` to `take_legacy_reconciliation_store()` (read + delete the file,
   version-checked, posture-ungated), keep `reconciliation.json` in the v1→v2 legacy move list.
5. `smelt-runtime/src/execute.rs`: replace the `execute.rs:3652` JSON block with the engine reset
   routed through the new `Backend` hook so it commits with the batch's own region write; add the
   once-per-run legacy import before the first maintenance write.
6. Non-DuckDB dialects: no frontier builder exists (a Spark ledger builder is explicitly out of
   scope for this outcome), so the record is skipped with a `tracing::warn!` naming the model and
   the missing builder, and the legacy file is left in place. Phase 5 converts that warn into the
   recorded, explain-visible `MaintenanceStateDowngraded`; leave a comment saying so.
7. Update the doc comments that name the `.smelt/` residency as a live divergence
   (`file_store.rs:567`, `reconciliation.rs` module header, `run_state.md` references).

## Verification

- `bash .claude/scripts/verify-phase.sh` (full)
- `cargo test -p smelt-state --test reconciliation` and `cargo test -p smelt-state --lib`
- `cargo test -p smelt-backend-duckdb --lib`
- `cargo test -p smelt-runtime --test frontier_residency --test state_posture --test statement_parity --test execute_parity --test keyed_reprocessed_window_refusal`
- `cargo test -p smelt-cli --test maintenance_conformance` (the equivalence oracle — the ledger
  move must not shift any maintained model's result)
- `cargo test -p smelt-logical --test walk_coverage`
- Confirm `.claude/hardening-baseline.txt` is unchanged (or moved only via
  `hardening-budget.sh --update` with the reason recorded in the summary).

## Commit message

`feat(state): move the reconciliation ledger's frontier record engine-resident`
