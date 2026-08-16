# Phase 6 summary — `DeclaredContractRequiresState`

## Shipped

- `docs/specs/state.md` §"Declarations stay fail-loud": corrected which frontier
  `contract.deferral` is measured against (interval ledger + landed-delta record, not the
  engine-resident frontier record) and named the diagnostic's real trigger (posture, or a
  backend with no realisation).
- `docs/specs/diagnostics.md` §"Maintenance plan": added the `DeclaredContractRequiresState`
  catalogue row.
- `smelt_logical::maintenance::availability`: new `StateStructure::IntervalFrontier` variant and
  `StateAvailability::interval_frontier` field (posture-gated, not backend-gated); both
  `resolve_state_availability` matches extended (unreachable arm — no `Technique` depends on it).
- `smelt_logical::contract::state_requirements` (new module): pure
  `required_state_structures(&ContractConfig)` and
  `validate_contract_state(&ContractConfig, &StateAvailability) -> Vec<ContractStateRefusal>`.
  `frozen_horizon` yields no requirement (phase 1's decision stands).
- `smelt_db::queries::maintenance::state_availability_for_project(backend, StateMode)`:
  `state_availability_for`'s sibling, adds `interval_frontier` gated on the effective posture.
- `DiagnosticCode::DeclaredContractRequiresState` (smelt-db) + LSP code-string mapping
  (`declared-contract-requires-state`).
- `check_file_diagnostics` wiring: resolves the effective posture (model `state:` if declared,
  else project's), loops `project_active_backends` (falling back to `["duckdb"]`), runs the
  validator, dedupes by declaration so multiple backends don't multiply the same refusal.
- `examples/timeseries/smelt.yml` gains `state: mode: intervals` (it declares
  `daily_event_counts_deferred`'s `contract.deferral`, so under the doctrine it must carry the
  state that measures it).

## Decisions

- Backend-loop dedup by declaration string (`BTreeSet`): `interval_frontier` availability
  depends only on posture, not backend, so looping every declared backend would otherwise emit
  one duplicate diagnostic per backend for the same declaration.
- Kept `unreachable!()` (not a new error path) for `IntervalFrontier` inside
  `resolve_state_availability`'s two exhaustive matches — `required_state_structure` (keyed by
  `Technique`) never returns it; only a declared contract point does, and that's a distinct
  validation path (`validate_contract_state`), never plan-derivation downgrade.

## For the next planner

- Row 7 (fuse the frontier reset into the region-recompute's own write transaction) and row 8
  (state-deletion conformance leg) are next; neither depends on this phase's additions.
- The `DeclaredContractRequiresState` message format (`"DeclaredContractRequiresState: {decl} —
  {why}"`) mirrors the existing `ContractDeferralInvalid`/`ContractFrozenHorizonInvalid`
  in-message-code-name convention in `check_file_diagnostics` — worth keeping consistent if a
  later phase adds more state-requiring lattice points.
- Not touched: `smelt explain`'s own contract-lattice rendering does not yet show
  `DeclaredContractRequiresState` refusals (it already didn't show `MaintenanceStateDowngraded`
  for real backends per phase 5's summary) — out of this phase's criterion 3 scope (that's the
  diagnostic surface, which is implemented and gated), but worth a follow-up if `smelt explain`
  becomes the primary discovery surface for this refusal.

## Gates

- `bash .claude/scripts/verify-phase.sh` (full) — PASS (fmt, clippy, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test contract_state_requirements --test contract_lattice_spec`
  — 5 + 13 passed.
- `cargo test -p smelt-db --test contract_deferral_diagnostics --test integration` — 8 + 363
  passed (includes the catalogue-coverage gate).
- `cargo test -p smelt-cli --test example_diagnostics --test maintenance_conformance` — both
  green (119 + 70 passed, combined run above).
- `cargo test -p smelt-lsp --test example_workspaces` — 34 passed.
- `cargo test -p smelt-runtime --test contract_deferral_skip_e2e --test
  declared_contract_requires_state_e2e` — 2 + 1 passed.
