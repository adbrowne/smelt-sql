# Phase 3 summary — key-valued dirt-sets through the graph layer

## Shipped

- `KeyValues` enum (`Resolved(Vec<String>)` / `Unresolved { reason }`) and `KeyedDirt::values`
  in `crates/smelt-logical/src/maintenance/propagate.rs` — the graph layer's keyed channel now
  carries resolved key values, not just key columns and provenance.
- `Edge::consumer_key_scope` (+ `with_consumer_key_scope` builder) — a consumer's own declared
  key restriction, consulted by the new pure `project_key_values` projection rule.
- `propagate_with_keys(edges, source_deltas, keyed_seeds)`; `propagate` is now a delegating
  wrapper over it with an empty seed map (every existing caller/test unchanged, pinned by
  `propagate_without_keyed_seeds_is_unchanged`).
- Fixed the keyed-only-node visit gap: a node with keyed dirt but no interval dirt is now
  visited so its outbound keyed edges compose (`key_values_compose_through_a_two_hop_keyed_chain`
  pins a two-hop A→B→C chain).
- `smelt-runtime::propagation`: `ClampAndLocality::key_scope_by_model` folds `PlanCell::key_scope`
  (phase 2's dispatch substitution) once per model; `build_forward_graph` attaches it to every
  inbound edge as `consumer_key_scope`. New `plan_since_upstream_with_keyed_seeds` (mirrors
  `plan_since_upstream_with_observed_deltas`'s shape); `SinceUpstreamPlan` gained `keyed_dirty`
  and the dirty-set report now renders the keyed channel alongside the interval channel.
- Narrowed the Known Divergences bullet in `docs/specs/incremental_models.md` to the still-open
  live-seed half (no spec re-word beyond that, per the plan).

## Decisions

- Seed lookup for an outbound keyed edge checks `keyed_seeds` first (the caller's own seed for
  that node), else falls back to the merged `keyed_dirty` already recorded for that node — this
  is what makes a keyed chain compose without threading the original seed map past the first hop.
- Mismatched key scope widens to whole-model dirt even into a keyed-grain consumer (previously
  only a clocked consumer of a keyed origin was ever widened) — matches the plan's explicit
  "even for a keyed-grain consumer" test requirement.
- An *absent* seed (no entry in the seed map) does not trigger the mismatch-widen path — it
  widens at dispatch time instead (phase 4+), per the plan's "Unresolved seeds" carve-out.

## For the next planner

- Phase 4 (dispatch composition in the run loop) can now read `SinceUpstreamPlan::keyed_dirty`
  and `Propagation::keyed_dirty`/`per_edge_keys` directly — the resolved values are there, just
  not yet consumed by the run loop's own cell substitution.
- Live keyed-seed resolution (reading the actual changed key values off the backend sidecar) is
  still open — `plan_since_upstream_with_keyed_seeds` takes an already-resolved seed map; nothing
  yet constructs one from a real run. That's phase 5's "Live consumption" row, per the outcome's
  own phase-3-planning decision log entry.
- Not touched: fan-in merge behavior in `merge_keyed_values` for a node with more than one
  admitted inbound keyed edge is implemented (union of resolved values, or `Unresolved` if any
  contributor is unresolved) but has no dedicated test — no fixture in this phase's test list
  exercised fan-in. Worth a targeted test once a real fan-in scenario shows up (fan-out is
  common in the tests; fan-*in* to a keyed consumer is not yet exercised).

## Gates

- `cargo test -p smelt-logical --test keyed_dirt_values --test maintenance_propagation_adjoint` — pass
- `cargo test -p smelt-runtime --test since_upstream_propagation --test typed_edge_graph --test key_addressed_model_edge_lowering` — pass
- `cargo test -p smelt-logical --test walk_coverage` — pass
- `cargo test -p smelt-cli --test maintenance_conformance` — pass (76 tests)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace test, example_diagnostics)
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches
