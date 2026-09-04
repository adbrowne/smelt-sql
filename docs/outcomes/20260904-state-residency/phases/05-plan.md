# Phase 5 plan — wire availability resolution into the plan-derivation seam

## Objective

Make every `smelt-runtime` maintenance-plan consumer read an *availability-resolved* plan:
one seam wraps `smelt-db`'s `derive_model_maintenance_plan{,_with_edges}` and applies
`resolve_availability` before any caller sees a cell. The non-DuckDB reconciliation-ledger skip
in `execute.rs` stops being a `RunReporter::state_structure_unavailable` stand-in and becomes an
availability-driven decision backed by the recorded `state_downgrade`. Advances criteria 3 and
(by retiring the stand-in) 7; sets up phase 6's surface work.

## Spec delta

None. `state.md` §"The degradation contract" already states the two-step shape normatively
(phase 1); this phase is wiring, no user-visible surface change. The user-visible channel for a
recorded downgrade (`smelt explain`, the warning diagnostic) is phase 6.

## Tests

`crates/smelt-logical/tests/maintenance_availability.rs` (extend):
- `duckdb_realises_every_state_structure` — `realisable_state_structures(SqlDialect::DuckDB)`
  returns all four variants.
- `a_ledger_less_dialect_realises_no_ledger` — Spark/BigQuery realise neither `MergeLedger` nor
  `ReconciliationLedger` (the builders are `smelt-state`'s `ddl_duckdb.rs` only), and still
  realise `FingerprintSidecar`/`ObservedOutputDeltas`.

`crates/smelt-runtime/tests/availability_seam.rs` (new):
- `availability_for_run_intersects_dialect_and_warehouse_tables` — DuckDB + `allowed` → all four;
  DuckDB + `none` → empty; Spark + `allowed` → no ledgers.
- `derive_resolved_downgrades_a_ledger_backed_cell` — a keyed-fold model derived through the seam
  under ledger-less availability yields a cell whose technique is the recompute equivalent and
  whose `state_downgrade` names `ReconciliationLedger`.
- `derive_resolved_with_edges_downgrades_a_column_merge_cell` — same for the edge-aware entry
  point and `MergeLedger`.
- `derive_resolved_under_full_availability_is_byte_identical_to_the_raw_derivation` — resolution
  is a no-op when everything is available (no behaviour change on DuckDB).
- `every_runtime_derivation_goes_through_the_availability_seam` — structural: `rg` over
  `crates/smelt-runtime/src/**` finds no `smelt_db::queries::maintenance::derive_model_
  maintenance_plan` call outside the seam module.

`crates/smelt-runtime/tests/statement_parity.rs` (amend):
- `ledger_reset_is_skipped_on_a_non_duckdb_dialect` — rewritten to assert the skip is driven by
  availability and that **no** `state_structure_unavailable` reporter event is emitted.
- `ledger_recompute_reset_statements_come_from_the_state_builder` / `delta_restricted_recompute_
  records_the_ledger_reset` — unchanged, guarding the DuckDB path.

## Tasks

1. Add `realisable_state_structures(dialect: SqlDialect) -> Vec<StateStructure>` to
   `crates/smelt-logical/src/maintenance/availability.rs` — exhaustive `match` over `SqlDialect`
   (a new dialect is a compile error, not a silent default), doc-commented with why the ledgers
   are DuckDB-only today (`smelt-state/src/ddl_duckdb.rs` is the only builder).
2. New `crates/smelt-runtime/src/maintenance_availability.rs`: `availability_for_run(dialect,
   &smelt_core::config::Config) -> StateAvailability` (`StateAvailability::resolve` over
   `config.state.warehouse_tables` and task 1's set), plus `derive_resolved(...)` /
   `derive_resolved_with_edges(...)` — same argument lists as the `smelt-db` functions plus
   `&StateAvailability`, calling through and then `resolve_availability(&mut result.cells, …)`.
   Module doc names it the single derivation seam for `smelt-runtime`.
3. Route every production `derive_model_maintenance_plan{,_with_edges}` call in
   `maintenance_driver.rs`, `propagation.rs` and `execute.rs` through the seam; add a
   `availability: &StateAvailability` parameter to each affected `resolve_*` resolver and thread
   it from the `execute.rs` call sites (which already hold `backend.dialect()` and `Config`).
4. Build the run's `StateAvailability` once per `execute_project` run, next to where the target
   `Config` and backend are resolved, and pass it down; do not rebuild it per model.
5. `execute.rs` ledger-reset site: replace the `backend.dialect() == SqlDialect::DuckDB` guard
   with `availability.contains(StateStructure::ReconciliationLedger)`, delete the
   `reporter.state_structure_unavailable(...)` stand-in call there, and leave a `tracing::debug!`
   plus a comment pointing at the recorded `state_downgrade` as the user-visible channel
   (surfaced in phase 6). Leave the `RunReporter` method and its keyed-grain caller alone —
   phase 6 removes those.
6. Update `docs/specs/incremental_shapes.md` lines ~1251/~1396 only if they now misdescribe the
   `execute.rs` site (the stand-in note); keep the keyed-grain mention until phase 6.
7. Write `phases/05-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_availability --test walk_coverage`
- `cargo test -p smelt-runtime --test availability_seam --test statement_parity --test execute_parity`
- `cargo test -p smelt-runtime --test keyed_frontier_bookkeeping`
- `cargo test -p smelt-cli --test maintenance_conformance`

## Commit message

`feat(state-residency): resolve availability at the single runtime derivation seam`
