# Phase 7 plan — the two state-residency `DiagnosticCode` variants

## Objective

Land `MaintenanceStateDowngraded` (Warning) and `DeclaredContractRequiresState` (Error) as real
`DiagnosticCode` variants, emitted from the single pure owner `maintenance_plan_diagnostics`
(`crates/smelt-db/src/queries/maintenance.rs`) so both the LSP and the CLI analyzer gate surface
them ahead of any run. Advances success criterion 4 (the diagnostic half — the `smelt explain`
half landed in phase 6) and criterion 5 (`warehouse_tables: none`'s only two consequences are
these codes).

## Spec delta

Both codes are already normative (`state.md` §Diagnostics, `diagnostics.md` §"State residency"),
so the delta is divergence bookkeeping only:

- `docs/specs/diagnostics.md` §Known divergences — delete the bullet "**Both state-residency codes
  are catalogue-ahead-of-variant.**" (both variants exist after this phase).
- `docs/specs/state.md` §Known Divergences (~line 297) — the bullet "No availability-resolution
  step exists in derivation" is stale on both halves after this phase (phases 4-6 landed the step;
  this phase lands the codes). Rewrite it to the residual gap only, or delete it if none remains.
  Phase 11 owns the remaining four bullets; do not touch them here.

## Tests

`crates/smelt-logical/src/contract/mod.rs` (unit):
- `deferral_requires_the_reconciliation_ledger` — `required_state_structure(&ContractPoint::Deferral{..}) == Some(ReconciliationLedger)`; `Default`/`FrozenHorizon` → `None`.

`crates/smelt-db/tests/maintenance_diagnostics.rs`:
- `keyed_fold_on_a_spark_target_warns_state_downgraded` — a `grain: key` fold model in a project whose only target is `type: spark` gets one Warning `MaintenanceStateDowngraded` naming the cell, the original technique, the missing structure and the backend.
- `keyed_fold_on_duckdb_emits_no_state_downgrade` — same model, `type: duckdb`, no such diagnostic.
- `warehouse_tables_none_warns_state_downgraded_on_duckdb` — `state:\n  warehouse_tables: none` forces the warning on DuckDB.
- `state_downgrade_warning_never_blocks` — the diagnostic's severity is `Warning`, so the analyzer gate still admits the model.
- `deferral_without_a_ledger_refuses_declared_contract_requires_state` — model-level `contract.deferral` on a Spark-only target ⇒ one Error `DeclaredContractRequiresState` naming the declaration and the missing structure.
- `cell_level_deferral_without_a_ledger_refuses` — same via `contract.cells[].deferral`.
- `deferral_on_duckdb_is_admitted` — same declaration on DuckDB with `warehouse_tables` default: no `DeclaredContractRequiresState`.

Gates that must stay green as tests: `cargo test -p smelt-db --test integration diagnostics_catalogue`
(enum → catalogue coverage now covers both new variants), `cargo test -p smelt-cli --test example_diagnostics`.

## Tasks

1. `smelt-logical`: add pure `contract::required_state_structure(&ContractPoint) -> Option<StateStructure>`, exhaustive over `ContractPoint` (contract-lattice single-ownership: the mapping is part of the point's definition, not a caller's guess). Red-green with its unit test.
2. `smelt-core`: add `parse_warehouse_tables(text: &str) -> Option<WarehouseTables>` beside `parse_active_backends` (same posture: parse `Config`, `None` on empty/unparseable text).
3. `smelt-db`: add `#[salsa::tracked] project_warehouse_tables(db, project)` in `queries/project.rs`, wrapping that parser over `ProjectInput::smelt_yml_text`.
4. `smelt-db` (`queries/maintenance.rs`): add `backend_dialect_for(backend_name: &str) -> Option<SqlDialect>` mirroring `backend_write_capabilities_for`'s name vocabulary; an unrecognised name resolves to no availability at all (conservative, documented — same posture as that function's default caps).
5. Extend `MaintenancePlanDiagnostics` with two Salsa-safe (`PartialEq`/`Eq`) vectors: `state_downgrades` and `contract_state_refusals`, each carrying the rendered message fields (cell label, original technique, missing structure, backend name / declaration label). Document them like the existing fields.
6. In `maintenance_plan_diagnostics` (new `warehouse_tables: WarehouseTables` parameter): after the plan is derived, for each declared backend build `StateAvailability::resolve(warehouse_tables, &realisable_state_structures(dialect))`, run `resolve_availability` over a **clone** of `result.plan.cells` (the returned plan/report must stay ideal-derivation output — the runtime and explain own their own resolution), and collect one downgrade record per cell, naming the first backend that downgrades it (`write_pin_diagnostics`'s one-per-cell posture).
7. Same function: for each declared `contract.deferral` (model-level and each `contract.cells[].deferral`), map the point through task 1's function and push a `contract_state_refusals` entry when the required structure is unavailable on any declared backend.
8. Thread `project_warehouse_tables` into the `maintenance_plan` Salsa wrapper (`smelt-db/src/lib.rs`) beside the existing `project_active_backends` call.
9. Add `DiagnosticCode::MaintenanceStateDowngraded` and `DiagnosticCode::DeclaredContractRequiresState` to `crates/smelt-db/src/diagnostics_types.rs` with doc comments matching `diagnostics.md`'s rows; fold both vectors into `check_file_diagnostics` next to `scan_bounds_warnings`/`write_pin_refusals` (Warning / Error respectively, anchored at the SQL body start).
10. Add the two kebab-case strings to `smelt-lsp/src/backend.rs`'s code mapping (compiler-enforced exhaustive match).
11. Apply the spec delta above.
12. Update `availability.rs`'s module doc, which still says "No consumer calls `resolve_availability` yet — that is a later phase's wiring seam" (false since phase 5).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-db --test maintenance_diagnostics --test integration`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-cli --test example_diagnostics --test explain_maintenance`
- `cargo test -p smelt-runtime --test availability_seam --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`

## Commit message

`feat(state-residency): MaintenanceStateDowngraded + DeclaredContractRequiresState diagnostics`
