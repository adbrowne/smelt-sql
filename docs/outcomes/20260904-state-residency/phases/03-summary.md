# Phase 3 summary — ledger statements under gate coverage; the delta-restricted write path recorded

**Shipped:**
- `crates/smelt-runtime/src/maintenance_driver.rs`: `execute_delete_insert_with_delta_restriction`
  gained `ensure_sqls: &[String]`/`pre_write_sqls: &[String]` parameters; its terminal write routes
  through `Backend::execute_write_with_bookkeeping` when either is non-empty, else the pre-phase-3
  `execute_statement_group` call (byte-identical for every existing caller passing `&[], &[]`).
- `crates/smelt-runtime/src/execute.rs`: the ledger reset construction (ensure DDL +
  `generate_ledger_recompute_reset_sqls`) is hoisted once per batch, above the three DuckDB
  DeleteInsert dispatch arms (model-edge restricted, external-sidecar restricted, plain) — all
  three now build the SAME reset and pass it through, instead of only the plain branch building
  one and the two restricted branches recording nothing (phase 2's surfaced gap). Skipped when a
  live `ColumnScopedMerge` cell dispatches (not a region DeleteInsert).
- Six new/extended tests: `statement_parity.rs`'s `ledger_recompute_reset_statements_come_from_
  the_state_builder`, `delta_restricted_recompute_records_the_ledger_reset`,
  `ledger_reset_rolls_back_with_a_failed_write`, `ledger_reset_is_skipped_on_a_non_duckdb_dialect`;
  `keyed_frontier_bookkeeping.rs`'s `merged_window_ledger_upsert_matches_the_state_builder` (plus a
  new `RecordingBackend`/`RecordingBackendFactory` pair in that file); `delta_restricted_recompute_
  statements_come_from_the_emitter` confirmed unchanged by the new parameters.
- All ~30 pre-existing direct callers of `execute_delete_insert_with_delta_restriction` across
  `smelt-runtime`/`smelt-cli` tests updated to pass `&[], &[]`.

**Decisions:**
- The non-DuckDB skip test (`ledger_reset_is_skipped_on_a_non_duckdb_dialect`) uses a fully mocked
  `Backend` claiming `SqlDialect::SparkSQL` rather than a real Spark connection or a DuckDB backend
  with an overridden `dialect()` — the latter risks a genuine SQL-syntax mismatch between
  Spark-dialect-printed text and the real DuckDB engine underneath, unrelated to what the test
  checks (which SQL gets *built*, not whether it executes against a live warehouse).
- `merged_window_ledger_upsert_matches_the_state_builder` reads back the actual `_smelt_ledger`
  rows a run produced, then rebuilds the expected upsert from those values via
  `generate_ledger_upsert_sql`, rather than hand-predicting `step.partition_value`/`delta_id`
  formatting — more robust to the keyed-fold step internals than guessing their string shape.

**For the next planner:**
- Phase 4 (`MaintenanceStateDowngraded`) should replace the `state_structure_unavailable`
  reporter call this phase's hoisted block still makes on a non-DuckDB dialect —
  `ledger_reset_is_skipped_on_a_non_duckdb_dialect` locks today's shape so phase 4 has a red test
  to turn green.
- No other gaps surfaced. The delta-restricted/external-sidecar write paths now record the same
  ledger reset the plain path does; `.smelt/reconciliation.json` remains fully absent from
  production code.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  `cargo test`, example_diagnostics).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test
  keyed_frontier_bookkeeping` — 4 + 4 + 37 passed.
- `cargo test -p smelt-runtime --test delta_restricted_recompute --test region_conditional_write
  --test web_analytics_session_delta_restriction` — 4 + 2 + 2 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 75 passed.
- `rg -n 'reconciliation\.json' crates/` — hits only comments/doc-prose and the legacy-migration
  test fixture string; no production reader/writer.
