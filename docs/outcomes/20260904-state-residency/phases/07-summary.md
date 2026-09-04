# Phase 7 summary — the two state-residency `DiagnosticCode` variants

## Shipped

- `MaintenanceStateDowngraded` (Warning) and `DeclaredContractRequiresState` (Error) are real
  `DiagnosticCode` variants (`crates/smelt-db/src/diagnostics_types.rs`), folded into
  `check_file_diagnostics` (`crates/smelt-db/src/lib.rs`) and mapped to kebab-case LSP codes
  (`crates/smelt-lsp/src/backend.rs`).
- `smelt_logical::contract::required_state_structure(&ContractPoint)` — pure, exhaustive,
  single-owner mapping (`crates/smelt-logical/src/contract/mod.rs`): `Deferral` requires
  `ReconciliationLedger`; `Default`/`FrozenHorizon` require nothing.
- `smelt_core::parse_warehouse_tables` (beside `parse_active_backends`) and the Salsa wrapper
  `project_warehouse_tables` (`crates/smelt-db/src/queries/project.rs`).
- `backend_dialect_for` (`crates/smelt-db/src/queries/maintenance.rs`) — declared backend name →
  `SqlDialect`, mirroring `backend_write_capabilities_for`'s vocabulary; unrecognised name → no
  availability.
- `maintenance_plan_diagnostics` now resolves availability per declared backend (over a **clone**
  of the derived cells — the returned plan/report stays ideal-derivation output) and populates two
  new `MaintenancePlanDiagnostics` fields, `state_downgrades` and `contract_state_refusals`.

## Decisions

- Availability is checked against every declared backend, the same all-declared-backends posture
  `write_pin_diagnostics` already uses; one diagnostic per cell/declaration, naming the first
  backend that fails.
- The `ContractPoint::Deferral { d }` payload is irrelevant to `required_state_structure` (dispatch
  is on the variant), so the diagnostics-assembly caller uses a placeholder `d: 0`.
- `examples/timeseries/smelt.yml` had a second `spark` target declared purely for illustration
  (never exercised by any test with `--target spark`). Real availability resolution now correctly
  flags three models on that project that need DuckDB-only ledger structures — removed the spark
  target rather than suppress the (correct) new diagnostics; trimmed the matching stale sections
  from `examples/timeseries/README.md` (which already called Spark "a stub implementation" —
  pre-existing staleness, not something this phase should expand scope to fully rewrite).

## For the next planner

- `docs/specs/state.md` §Known Divergences bullet "`state.warehouse_tables` is unimplemented" is
  now stale on both its claims (parsing exists via `parse_warehouse_tables`; availability
  resolution exists since phases 4-6) — explicitly left untouched per this phase's plan ("phase 11
  owns the remaining four bullets"), only its dangling cross-reference to the deleted sibling
  bullet was repointed. Phase 11 should rewrite or delete it along with the other three.
- No new gaps surfaced beyond what the plan anticipated.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature sets,
  full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-db --test maintenance_diagnostics --test integration diagnostics_catalogue`
  — green (38 + 1 tests).
- `cargo test -p smelt-logical --test walk_coverage` — green.
- `cargo test -p smelt-cli --test example_diagnostics --test explain_maintenance` — green (119 + 28).
- `cargo test -p smelt-runtime --test availability_seam --test statement_parity --test execute_parity`
  — green (6 + 4 + 37).
- `cargo test -p smelt-cli --test maintenance_conformance` — green (75).
