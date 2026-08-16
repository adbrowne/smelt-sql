# Phase 6 summary — Key-addressed model-edge cells (plan derivation)

## Shipped

- `docs/specs/incremental_models.md` §"Upstream model edges": a `KeyedUpsert`-shaped upstream
  contributes a key-addressed edge — no clock required, a keyed-grain downstream admitted too,
  fail-closed refusal named explicitly when the downstream doesn't carry the upstream's keys.
  §"Known Divergences" gains a bullet narrowing the deferred-lowering gap.
- `smelt_logical::maintenance::derive::ModelEdge` gains `output_shape: Option<OutputDelta>` —
  the upstream's own derived output-delta shape, scalar per edge
  (`crates/smelt-logical/src/maintenance/derive.rs`).
- `smelt_logical::maintenance::{PlanCell, KeyScope}`: `PlanCell` gains `key_scope:
  Option<KeyScope>` (`{ keys, from }`), the key-addressed read restriction alongside `scans`
  (`crates/smelt-logical/src/maintenance/mod.rs`).
- `smelt_logical::maintenance::repair::admit_key_addressed_recompute` — sibling of
  `admit_per_group_recompute`, reusing `derive_affected_keys` with the upstream's key columns
  as the `DeltaShape`; refuses `KeysNotDiscoverable`/`SliceUnbounded` by name.
- `append_model_edge_cells` (`derive.rs`): a new loop, run before the clock/`output_partition_col`
  gates, admits a key-addressed `PerGroupRecompute` cell for any `KeyedUpsert`-shaped edge the
  existing clock-based `DeleteInsert` route wouldn't otherwise serve (clockless upstream, or a
  keyed-grain downstream) — narrowing `ReachNotDerivable`, never removing it for the remaining
  (non-key-addressed) clockless case.
- `smelt-runtime::propagation::derive_clamp_and_locality{,_pass}` now derives each model edge's
  `output_shape` from the same `workspace_output_delta_verdicts`/`upstream_output_delta_groups`
  fold `build_forward_graph`'s `type_edge` call already used (meet-folded to one scalar per
  edge) — no second, independent derivation.
- Bug fix: `analysis::fingerprint::relation_matches_source` only stripped a `sources.` breadcrumb,
  never `models.` — so any repair-family proof over a `smelt.models.<addr>`-style model ref
  (the common case for a model living directly under `models/`) always resolved to "touches no
  columns", making affected-key discovery fail closed for every such edge. Now strips either
  breadcrumb; provably additive (a relation carries at most one breadcrumb), full workspace test
  suite green afterward.
- Tests: `crates/smelt-logical/tests/keyed_model_edge.rs` (5 new: admission, keyed-consumer,
  narrowed refusal, no-interval-scan honesty, missing-keys refusal) and
  `crates/smelt-runtime/tests/typed_edge_graph.rs::clockless_keyed_upstream_is_walkable_on_the_real_graph`
  (the phase-5 fixture's clock removed — exercises the real graph builder end to end).

## Decisions

- The key-addressed route is taken only when the existing clock-based route has nothing to admit
  (`edge.clock_col.is_none() || output_partition_col.is_none()`), not unconditionally for every
  `KeyedUpsert` edge. An unconditional rule broke every existing clocked-`KeyedUpsert`-upstream
  fixture: those downstreams never declare a `unique_key`/provable grain (the old `DeleteInsert`
  route needs neither), so routing them through `admit_key_addressed_recompute` refused them
  outright instead of switching technique. Both routes are admissible for that shape; narrowing
  which edges move keeps every pre-existing fixture's technique stable — see outcome.md decision
  log.
- `output_shape` is a scalar per edge (the meet across `upstream_output_delta_groups`'s
  per-column-group verdicts), not per-column-group like `type_edge`'s edge-typing proof. This
  admission gate only decides whether to attempt the key-addressed route at all — coarsening
  here only ever narrows (never widens) which edges get the new route, unlike the per-column
  decision the 2026-08-09 log entry protects.
- `JoinContext` gained `#[derive(Clone)]` (previously `Debug, Default` only) so
  `admit_key_addressed_recompute` can pass the caller's already-built context by value into
  `AffectedKeyContext` without a second empty context.

## For the next planner

- **Lowering is untouched, as scoped** — a key-addressed `PlanCell` has no statement emitter, no
  driver dispatch. Phase 7's job, per the outcome's own phase-6 reshape note.
- **`smelt explain` doesn't render key-addressed model-edge cells yet** — `smelt-db::lib.rs`'s
  `ref_model_edge` (the Salsa-side `ModelEdge` builder `smelt explain` uses) still sets
  `output_shape: None` unconditionally; wiring the workspace-wide output-delta fold into that
  Salsa query is real work (Salsa-purity-compatible input gathering, workspace-wide computation)
  and is phase 9's own surface scope, not silently done here.
- `relation_matches_source`'s `models.`-breadcrumb gap almost certainly affects
  `model_edge_enrichment_closure`/`find_enrichment_join` in `skeleton_closure.rs` too (same
  `sources.`-only stripping pattern, same doc-comment cross-reference) — not touched this phase
  because no fixture exercises a `smelt.models.<addr>` ref inside a JOIN (every existing
  enrichment-closure test uses a `smelt.<namespace>.<addr>` address). Flagging as a latent gap,
  not fixed speculatively.
- `execute.rs`'s own `ModelEdge` construction (the live runtime driver's edge list, distinct from
  `propagation.rs`'s graph-builder one) still sets `output_shape: None` — correct for this
  phase's plan-derivation-only scope, but phase 7 will need to wire it the same way phase 6 wired
  `propagation.rs`, or the driver will never actually dispatch a key-addressed cell it derives.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test keyed_model_edge --test model_edge_delta_restriction --test repair_cell --test maintenance_plan_refusals --test maintenance_coverage_matrix --test maintenance_propagation_adjoint --test walk_coverage` — all green, no fixture changes needed.
- `cargo test -p smelt-runtime --test typed_edge_graph --test since_upstream_propagation --test statement_parity --test execute_parity` — all green.
- `cargo test -p smelt-cli --test maintenance_conformance --test explain_maintenance` — all 63 + 11 green.
