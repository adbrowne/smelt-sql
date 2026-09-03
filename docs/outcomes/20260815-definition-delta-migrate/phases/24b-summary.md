# Phase 24b summary — bare-keyed→reader model-edge admission (`RepairKeysNotDiscoverable`)

**Shipped:**
- `crates/smelt-logical/src/maintenance/repair.rs::admit_key_addressed_recompute` now attempts a
  second discovery route (**grain-over-upstream**) when the upstream-keyed route (route 1) fails:
  it derives the downstream's read-column set off the upstream via `fingerprint_projection`,
  gates explicitly on no fan-out join (`model_property_vector(sql, join).has_fan_out_join`), and
  re-runs `derive_affected_keys` against that column set.
- `smelt_logical::maintenance::{KeyScope, KeyDiscovery}` (`crates/smelt-logical/src/
  maintenance/mod.rs`): `KeyScope` gains a `discovery: KeyDiscovery` field
  (`UpstreamKeyed`/`DownstreamGrainOverUpstream`) — plan data, not re-derived downstream.
- `crates/smelt-runtime/src/maintenance_driver.rs`: `resolve_live_key_addressed_model_edge_cell`'s
  `MaintenanceKeyScopeColumnMissing` subset check now gates on `UpstreamKeyed` only; the function
  also resolves and returns a `group_key` (the sidecar's own grouping key — `upstream_keys` for
  route 1, `key_scope.keys` for route 2). `resolve_key_addressed_affected_keys` and
  `execute_key_addressed_model_edge_cell` take an explicit `group_key`/`discovery` pair; for
  `DownstreamGrainOverUpstream` the forward-projection `SELECT` is skipped entirely — the
  sidecar's own diffed keys become the affected-key relation directly
  (`repair_keys_literal_select`), since they're already at the downstream's own grain.
- `crates/smelt-cli/src/explain.rs`: the repair stanza's "affected-key discovery" line now names
  which route a cell took.
- Spec: `docs/specs/incremental_models.md` §"Upstream model edges" — two named discovery routes,
  the corrected admission-refusal wording (grain resolvable against the upstream *relation*, not
  "carries the upstream's key columns"), and the moved-value under-approximation rationale for
  why route 2 never uses route 1's forward projection. §Known Divergences' "Graph-layer gaps"
  bullet narrowed to the one surviving clause (key-temporal-locality establishment, unrelated to
  this admission).
- Tests: 3 new `crates/smelt-logical/tests/keyed_model_edge.rs` cases
  (`grain_over_upstream_columns_is_admitted`, `grain_from_another_relation_is_still_refused`,
  `fan_out_join_blocks_the_grain_route`) plus a retargeted `consumer_not_carrying_upstream_keys_
  is_refused`; 3 new `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs` cases
  (`grain_route_groups_sidecar_at_downstream_grain`, `equal_key_route_is_unchanged`, and the live
  DuckDB `moved_grain_value_repairs_both_groups` proving both the vacated and arriving group
  recompute); `crates/smelt-cli/tests/explain_maintenance.rs::device_user_edges_admits_a_key_
  addressed_cell` against the real `examples/web_analytics`; extended `since_upstream.rs`'s
  whole-workspace flagship to assert `silver.device_user_edges` is in the RUN set.

**Decisions:**
- See outcome.md's 2026-09-03 (plan 24b) decision-log entry (written at plan time, matched
  verbatim by the implementation): group the sidecar at the downstream's own grain for route 2
  rather than the upstream's key, so a moved grain value flips both groups' XOR digests.
- `consumer_not_carrying_upstream_keys_is_refused`'s fixture was retargeted to an opaque-function
  grain expression (both routes fail closed to the same reason) rather than its old "downstream
  reads a column the upstream doesn't carry" shape — that shape is now legitimately admissible
  under route 2, per plan instruction.

**For the next planner:**
- `RUN silver.device_user_edges: keyed (keys: event_id)` already appeared in the propagation
  dry-run report *before* this phase (propagation's keyed-dirt classification doesn't depend on
  `admit_key_addressed_recompute` succeeding) — the outcome.md phase-24 decision-log's "silently
  unscheduled" wording refers to the maintenance *plan* having no cell at real dispatch time, not
  the dry-run report. Worth a clarifying note if a future reader trips on this.
- No other residue surfaced; the three verified clauses of the old "Graph-layer gaps" bullet
  collapsed to one (key-temporal-locality establishment) as expected.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-logical --test keyed_model_edge` — 8 passed
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering` — 12 passed
- `cargo test -p smelt-cli --features duckdb --test explain_maintenance --test since_upstream` — 21 + 19 passed
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance` — 74 passed
- `cargo test -p smelt-runtime --test statement_parity` — 25 passed
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed
