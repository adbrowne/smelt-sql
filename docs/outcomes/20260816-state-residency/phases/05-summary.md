# Phase 5 summary — two-step ideal-then-availability derivation

## Shipped

- `smelt_logical::maintenance::availability` (new module): `StateStructure`
  (`ReconciliationLedger` | `FrontierRecord`), `StateAvailability` (with `all()`/`none()`),
  `StateDowngrade`, `required_state_structure(Technique)`, and the pure
  `resolve_state_availability(&MaintenancePlan, &StateAvailability) -> ResolvedPlan` — a
  ledger-needing cell with a `recompute_fallback` downgrades to it; one with none pushes a
  `Refusal::NoAdmissibleTechnique` and is dropped; a frontier-needing cell keeps its technique
  and records an advisory-only downgrade. The ideal plan passed in is never mutated.
- `PlanCell::recompute_fallback: Option<RecomputeFallback>` (additive field, ~25 call sites
  updated): populated only at the `Technique::KeyedFold` push site in `derive.rs` by calling
  the existing `repair::admit_per_group_recompute` with the inputs already in scope.
- `smelt-db`: `MaintenancePlanResult` gains `ideal_plan`/`state_downgrades`;
  `derive_model_maintenance_plan`/`_with_edges` gain a `StateAvailability` parameter and resolve
  internally (edges variant resolves *after* appending model-edge cells, so a clocked edge's own
  `DeleteInsert` creation cell is covered too). `state_availability_for(backend_name)` mirrors
  `backend_write_capabilities_for` (DuckDB → `all()`, everything else → `none()`).
  `state_downgrade_diagnostics(ideal_plan, active_backends)` resolves per declared backend and
  folds into `MaintenancePlanDiagnostics::state_downgrades`, wired into `file_diagnostics()` as
  warning-severity `DiagnosticCode::MaintenanceStateDowngraded`.
- `smelt explain`: a downgraded cell prints a `state downgrade: <resolved> (ideal: <ideal>,
  missing <structure>) — <why>` line beside its `technique:` line.
- `docs/specs/diagnostics.md` §"Maintenance" catalogue row for the new code.
- Retired the phase-4 `tracing::warn!` at the frontier-skip site in `execute.rs` (demoted to
  `debug!`) — the same fact is now derived and reported ahead of the run as the diagnostic above.

## Decisions

- Threaded `StateAvailability` as a real parameter into the two `smelt-db` derive functions
  (not hidden behind a default), matching the outcome's decision log — but only the two runtime
  call sites that already carry `dialect: SqlDialect` in scope
  (`resolve_live_per_group_recompute_cell`, `resolve_live_key_addressed_model_edge_cell`) pass
  real availability today; every other call site (13 tests, 5 other `maintenance_driver.rs`
  resolvers, `propagation.rs`, `smelt-db`'s own `maintenance_plan_diagnostics`/`lib.rs` explain
  path) passes `StateAvailability::all()` — identical behaviour to before this phase. See "For
  the next planner" below for what that leaves undone.
- `state_downgrade_diagnostics` re-resolves the ideal plan per `active_backends` entry rather
  than reusing whatever `derive_model_maintenance_plan` was itself called with — mirrors
  `write_pin_diagnostics`'s existing per-backend loop exactly, so a project with more than one
  declared target backend gets a downgrade reported for each one that lacks the structure, not
  just the caller's single default.
- Excluded `MaintenanceStateDowngraded` from the example-workspace zero-diagnostics gates
  (`smelt-cli/tests/example_diagnostics.rs`, `smelt-lsp/tests/example_workspaces.rs`): every
  `examples/timeseries`/`examples/multi_engine` model that declares `spark` as an active backend
  now legitimately downgrades (no Spark frontier builder exists, and none is in scope for this
  outcome) — a real, expected advisory, not a regression signal. Every other diagnostic code
  still asserts zero for these fixtures.

## For the next planner

- **The runtime execution path mostly still reads the ideal plan.** Only the two dialect-aware
  `maintenance_driver.rs` resolvers actually downgrade a cell before dispatch; the other five
  resolvers (`resolve_incremental_strategy`, `resolve_live_column_scoped_cell`,
  `resolve_live_in_place_update_cell`, `resolve_live_membership_recompute_cell`,
  `resolve_live_delta_restriction_facts`) and `propagation.rs` have no backend name plumbed to
  them at all today, so they pass `all()`. In practice this is safe for now — every existing
  runtime backend is DuckDB, and `execute.rs`'s own frontier-skip site (independently) already
  degrades gracefully on a non-DuckDB write — but a future Spark execution path would need those
  five resolvers' own callers to plumb a real dialect through before the availability parameter
  does anything for them. Not scoped into this phase (the plan's own decision log only committed
  to resolving *inside* the two `smelt-db` functions, not to wiring every runtime call site).
- **`smelt explain`'s report path (`maintenance_plan_report`) always resolves against `all()`.**
  `smelt explain` therefore never shows a downgrade for a project's real declared backends today
  — only a caller that builds its own `MaintenancePlanResult` (as the new CLI test does) can
  exercise the print path. Making `smelt explain` genuinely backend-aware (reading
  `active_backends` the way `maintenance_plan_diagnostics` already does, and either resolving
  against the first one or printing per-backend) is a reasonable phase-9 (docs/validate) or
  fast-follow item — flagging it here since criterion 3 asks for "explain-visible" and today
  that's only true when a `MaintenancePlanResult` is hand-built with real downgrades already on
  it, not through the CLI's own resolution path.
- **`KeyedFold`'s `recompute_fallback` did not come out `Some` for the simplest possible test
  fixture** (`SELECT user_id, SUM(amount) ... GROUP BY user_id` over a single clocked
  append-only source, no `WHERE`) — `admit_per_group_recompute`'s bounded-read obligation
  (obligation 4) needs a real predicate link to the output axis that a bare `GROUP BY` doesn't
  provide. The unit tests in `crates/smelt-logical/tests/state_availability.rs` exercise the
  fallback-present and fallback-absent paths directly with hand-built cells, so the pure
  resolution logic is proven either way; the `smelt-db` integration test
  (`state_downgrade_surfaces_as_an_advisory_diagnostic`) ended up demonstrating the
  `DeleteInsert`/`FrontierRecord` downgrade end-to-end instead of the `KeyedFold` one. A future
  phase wanting an end-to-end `KeyedFold`-downgrade fixture will need a model shape that
  genuinely admits the repair fallback (a windowed `WHERE` clause, most likely).
- Row 6 (`DeclaredContractRequiresState`) and row 7 (frontier-fusion) are unaffected by this
  phase's shape and can proceed as planned.

## Gates

- `bash .claude/scripts/verify-phase.sh` (full): **PASS**.
- `cargo test -p smelt-logical --test state_availability --test maintenance_plan_admission --test maintenance_plan_conformance --test walk_coverage`: **PASS** (20 tests).
- `cargo test -p smelt-db --test maintenance_diagnostics --test integration`: **PASS** (371 tests, including the diagnostics-catalogue coverage gate).
- `cargo test -p smelt-cli --test explain_maintenance --test maintenance_conformance`: **PASS** (91 tests).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test technique_lowering`: **PASS** (57 tests).
- `cargo test -p smelt-lsp --test example_workspaces` (companion parity gate, not in the plan's own list but re-verified since it also asserts zero example diagnostics): **PASS** (34 tests).
- `.claude/hardening-baseline.txt`: unchanged (no regression).
