# Phase 4 summary — availability resolution as a pure derivation step

## Shipped

- `crates/smelt-core/src/config.rs`: `WarehouseTables { Allowed (default), None }` +
  `StateConfig::warehouse_tables`, `deny_unknown_fields`-shaped strict parsing (3 tests).
- `docs/specs/smelt_yml.md` §"Top-level keys": the `state` row now names `warehouse_tables:`
  alongside `mode:`, pointing at `state.md` for semantics/consequence.
- New `crates/smelt-logical/src/maintenance/availability.rs`, exported from `maintenance/mod.rs`:
  `StateStructure` (4 variants matching `state.md`'s inventory table), the exhaustive
  `required_state_structure(Technique) -> Option<StateStructure>`, `StateAvailability`
  (`all()`/`none()`/`resolve(WarehouseTables, &[StateStructure])`), `recompute_equivalent(&PlanCell)
  -> Technique`, and `resolve_availability(&mut [PlanCell], &StateAvailability)`.
- `PlanCell` gained `state_downgrade: Option<availability::StateDowngrade>`; all 28 existing
  `PlanCell` literals across `smelt-logical`/`smelt-runtime`/`smelt-cli` updated mechanically
  (`state_downgrade: None`).
- `crates/smelt-logical/tests/maintenance_availability.rs`: the plan's 9 named tests plus one
  extra (`a_cell_with_key_scope_downgrades_to_per_group_recompute`) — 10 total, all green.

## Decisions

- Availability resolution takes `&mut [PlanCell]` directly rather than `&mut MaintenancePlan`
  — it only ever touches cells, and taking the slice keeps the function trivially testable
  against hand-built cells without needing a full `MaintenancePlan` (refusals/key_locality are
  irrelevant to this step).
- `recompute_equivalent` checks `key_scope.is_some()` before matching on `corner` — a key-scoped
  cell is already `PerGroupRecompute` in practice, but this keeps the mapping honest per the
  plan's own three-way rule rather than relying on that coincidence.

## For the next planner

- **Unrelated pre-existing failure fixed as a scope exception**: `verify-phase.sh`'s full test
  run failed on `crates/smelt-cli/tests/partition_residue_probes.rs::
  partition_grain_residues_stay_closed` before any phase-4 code changed — commit `3e9c1a4a`
  (2026-09-04 decision track, `data_latency` retirement) edited `incremental_shapes.md`'s
  §"The partition grain" Known Divergences without updating this ratchet test's expected bullet
  list. Fixed here (removed the two stale expected leads, four remain) because it blocks the
  mandatory gate for every phase, not just this one, and required no design judgment — purely
  mechanical doc/test sync. The `decision-residue` outcome queued later in the backlog should
  double check no other ratchet tests still reference retired `data_latency` bullets.
- Phase 5 (wiring) is next: call `resolve_availability` at the seam where `smelt-runtime`'s
  maintenance driver calls `derive_model_maintenance_plan{,_with_edges}`, feed it a real
  `StateAvailability` built from the backend's realisable-structure set and
  `config.state.warehouse_tables`, and replace the keyed-grain `state_structure_unavailable`
  reporter call with a `state_downgrade`-driven path.
- No backend "realisable structures" enumeration exists yet — phase 5 will need one (probably a
  `Backend` capability query or a static per-backend table) to build the non-`None` half of
  `StateAvailability::resolve`'s `realisable` argument; this phase only built the pure
  intersection logic, not the backend-facing source of the DuckDB/Spark/BigQuery-specific set.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test suite, example_diagnostics).
- `cargo test -p smelt-logical --test maintenance_availability` — 10 passed.
- `cargo test -p smelt-core --lib config::` — 95 passed (includes the 3 new warehouse_tables
  tests).
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed, unchanged.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 41 passed,
  unchanged (this phase adds a step nothing calls yet).
