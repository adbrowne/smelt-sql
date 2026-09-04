# Phase 5 summary — availability resolution wired into the plan-derivation seam

## Shipped

- `smelt_logical::maintenance::availability::realisable_state_structures(dialect)` —
  exhaustive `SqlDialect` match; DuckDB realises all four `StateStructure`s, every other
  dialect realises the sidecar/output-delta pair only (`crates/smelt-logical/src/maintenance/availability.rs`).
- `crates/smelt-runtime/src/maintenance_availability.rs` (new module): `availability_for_run(dialect,
  config)` and the seam functions `derive_resolved`/`derive_resolved_with_edges` — thin
  availability-resolved wrappers over `smelt-db`'s raw derivation. This is now the ONLY place in
  `smelt-runtime` that calls `smelt_db::queries::maintenance::derive_model_maintenance_plan{,_with_edges}`
  (enforced by `availability_seam.rs`'s structural test).
- All 9 `maintenance_driver.rs` resolvers (`resolve_incremental_strategy`, `resolve_fold_deferral`,
  `resolve_live_column_scoped_cell`, `resolve_live_in_place_update_cell`,
  `resolve_live_membership_recompute_cell`, `resolve_live_per_group_recompute_cell`,
  `resolve_live_key_addressed_model_edge_cell`, `resolve_live_delta_restriction_facts`,
  `resolve_live_external_delta_restriction_facts`) take a new `availability: &StateAvailability`
  parameter and route through the seam instead of calling `smelt-db` directly.
- `execute.rs`: builds one `state_availability: HashMap<target, StateAvailability>` per run (pure
  `Config` facts, no live backend needed), right after `target_assignments` resolves — reused by
  the dry-run preview, the pre-loop deferral pass, and the real per-model loop alike (never
  rebuilt per model). New helpers `sql_dialect_for_target`, `availability_for_target`.
- `execute.rs` ledger-reset site (region-recompute reset before a DeleteInsert batch write): the
  `backend.dialect() == DuckDB` guard is now `availability.contains(StateStructure::ReconciliationLedger)`;
  the `reporter.state_structure_unavailable(...)` stand-in call is gone, replaced by a
  `tracing::debug!` pointing at the cell's own recorded `state_downgrade` as the real user-visible
  channel (phase 6 surfaces it).
- `propagation.rs`'s graph-walk call routes through `derive_resolved_with_edges` with
  `StateAvailability::all()` — that consumer only reads `trigger`/`scans`/`partition_local`/
  `key_locality`, never `technique`/`state_downgrade`, so full availability is behaviourally
  identical to a real resolution there; still routed through the shared seam so the structural
  rule holds without inventing a second, per-model-dialect-less derivation.
- New tests: `crates/smelt-logical/tests/maintenance_availability.rs` (+2:
  `duckdb_realises_every_state_structure`, `a_ledger_less_dialect_realises_no_ledger`);
  `crates/smelt-runtime/tests/availability_seam.rs` (new, 5 tests: `availability_for_run` intersection,
  a real `Technique::KeyedFold` downgrade through both `derive_resolved` and
  `derive_resolved_with_edges`, full-availability byte-identity, and the structural
  no-call-outside-the-seam test); `statement_parity.rs`'s
  `ledger_reset_is_skipped_on_a_non_duckdb_dialect` rewritten to assert **no**
  `state_structure_unavailable` call.

## Decisions

- Kept `warehouse_tables`/dialect availability as pure `Config` facts (no live `Backend` needed) so
  it could be built once, early, and reused by the dry-run path (which has no backend at all) —
  avoided threading a second, backend-derived availability value through the same call sites.
- `resolve_and_dispatch_key_addressed_edge_cell` (an `execute.rs`-private async helper, already
  taking `backend`+`config`) got `availability: &StateAvailability` threaded as an explicit
  parameter from both its call sites rather than recomputing it inline, keeping "build once" honest
  even for this indirect caller.
- `propagation.rs` did NOT get per-model dialect threading. Its plan-derivation call only reads
  fields `resolve_availability` never touches (confirmed by grep before writing the fix), so
  `StateAvailability::all()` is a documented, provably-safe no-op rather than a real gap — avoided
  a much larger refactor (this module has no per-model backend/dialect concept at all today; the
  fixture pool spans potentially cross-engine models) for zero behavioural benefit.
- The `derive_resolved_with_edges` seam test uses the same keyed-fold fixture (empty
  `model_edges`) rather than a live `ColumnScopedMerge`/`MergeLedger` admission — building a real
  dimension-join fixture that admits `ColumnScopedMerge` through the full pipeline needs
  structural machinery out of scope for a seam-wiring test; the pure-layer test in
  `crates/smelt-logical/tests/maintenance_availability.rs` already proves `resolve_availability`
  treats every ledger-requiring technique identically.

## For the next planner

- Phase 6 (surface): `smelt explain` must print `state_downgrade` (text + `--json`); a warning
  diagnostic must surface it in LSP/CLI; `DeclaredContractRequiresState` needs its validation
  refusal; the keyed-grain `state_structure_unavailable` skip (still called from one other site —
  grep `state_structure_unavailable` in `execute.rs` and `keyed_frontier`/repair callers) should be
  replaced by the recorded downgrade there too, per outcome criterion 3/7.
  `RunReporter::state_structure_unavailable` itself is NOT yet deleted — it's dead from the ledger
  ledger-reset site but still declared on the trait and still has its other keyed-grain caller;
  phase 6 owns removing it once that caller is gone too.
- `docs/specs/incremental_shapes.md` lines ~1251/~1396 (the stand-in note the phase 5 plan flagged
  to check) were NOT touched this phase — a quick read showed neither line references the
  `execute.rs` reporter stand-in by name, so nothing there was made stale by this change; worth a
  second look during phase 10's validate pass.
- Structural coverage note: `availability_seam.rs`'s `every_runtime_derivation_goes_through_the_availability_seam`
  greps `crates/smelt-runtime/src/**/*.rs` for the raw `smelt_db::queries::maintenance::derive_model_maintenance_plan`
  string, skipping comment-only lines and the seam module itself — a future call site added outside
  the seam will fail this test immediately, which is the enforcement the plan asked for.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature sets,
  full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-logical --test maintenance_availability --test walk_coverage` — 12 + 4 passed.
- `cargo test -p smelt-runtime --test availability_seam --test statement_parity --test execute_parity`
  — 5 + 37 + 4 passed.
- `cargo test -p smelt-runtime --test keyed_frontier_bookkeeping` — 4 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 75 passed.
