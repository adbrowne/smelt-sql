# Phase 3 plan — key-valued dirt-sets through the graph layer

## Objective

Make the graph layer's keyed channel carry **affected key values**, not just key columns and
provenance: `KeyedDirt` gains a resolved-values payload distinct from the unresolved-symbolic
form, `propagate` accepts keyed seeds as pure input, and composition projects an upstream's key
values onto each consumer's own key scope — widening to whole-model dirt when the projection
does not resolve. Advances success criterion 2 (key-valued dirt-sets end to end through the
graph layer) and lays the currency phase 4's multi-component dispatch consumes.

## Spec delta

None. §"Keyed dirt-sets and the narrowed refusal", §"Composition rules" and §"Unresolved seeds"
in `docs/specs/incremental_models.md` already pin exactly this behaviour (phase 1 landed them);
this phase makes the implementation match. Do **not** re-word the spec. The Known Divergences
bullet's "keyed dirt-sets carry key columns and provenance, not yet the affected key *values*"
clause is narrowed to the still-open live-seed half (live resolution is phase 5) — a
one-clause edit, not a new design.

## Design decisions (pin these; do not re-litigate)

- `KeyedDirt` grows `values: KeyValues`, where
  `enum KeyValues { Resolved(Vec<String>), Unresolved { reason: String } }`. `Resolved(vec![])`
  (nothing changed) and `Unresolved` are distinct — the spec's empty-vs-absent rule.
- Values are the same single-column `delta_key` strings the run-time mechanism already speaks
  (`maintenance_driver::diff_repair_group_sidecar_changed_keys` → `repair_keys_literal_select`),
  so a later phase can hand a propagated key set straight to the repair emitters.
- Propagation stays pure. New `propagate_with_keys(edges, source_deltas, keyed_seeds)`;
  `propagate` delegates with an empty seed map, so every existing caller and test is unchanged
  and an unseeded keyed edge propagates `Unresolved { reason }` (today's symbolic behaviour).
- Consumer key scope is a new `Edge` field `consumer_key_scope: Vec<String>` (default empty,
  builder `with_consumer_key_scope`). `smelt-runtime::propagation` fills it from the
  downstream's **already-derived** `PlanCell::key_scope` (`KeyScope::keys`) inside the existing
  per-model pass — never a second derivation (maintenance-plan purity).
- Projection rule (deliberately conservative, widen-never-narrow): values carry through when
  `consumer_key_scope` is non-empty and equals the upstream component's key columns as an
  ASCII-case-insensitive set; otherwise the consumer receives
  `Unresolved { reason }` naming the mismatch **and** whole-model interval dirt
  (`DayInterval::WHOLE`), including when the downstream is keyed-grain. An *unseeded* edge
  (unresolved seed) does **not** widen here — it widens at dispatch, per §"Unresolved seeds".
- A node with keyed dirt but no interval dirt must still be visited so its outbound keyed edges
  compose (today's `if node_dirty.is_empty() { continue; }` skips it — a real chaining gap).

## Tests

Unit/integration in `crates/smelt-logical` (new file
`crates/smelt-logical/tests/keyed_dirt_values.rs` unless a case is purely internal):

1. `seeded_key_values_reach_the_downstream_dirt_set` — a keyed seed on the upstream lands as
   `KeyValues::Resolved` in `keyed_dirty`/`per_edge_keys` for the consumer.
2. `empty_resolved_seed_is_not_an_unresolved_seed` — `Resolved(vec![])` propagates as
   `Resolved(vec![])`; an absent seed propagates `Unresolved` with a reason, and neither adds
   whole-model dirt.
3. `keys_that_do_not_project_onto_the_consumer_key_scope_widen_to_whole_model` — mismatched
   scope ⇒ `Unresolved { reason }` naming both key sets **plus** `DayInterval::WHOLE` dirt on
   the consumer, even for a keyed-grain consumer.
4. `key_values_compose_through_a_two_hop_keyed_chain` — A→B→C with matching scopes carries the
   values to C (pins the keyed-only-node visit fix).
5. `propagate_without_keyed_seeds_is_unchanged` — existing interval + symbolic-keyed results
   are byte-identical to today (regression pin for the delegating wrapper).
6. `crates/smelt-runtime/tests/since_upstream_propagation.rs`:
   `keyed_seed_values_flow_through_plan_since_upstream` — a keyed seed passed into the new
   `plan_since_upstream_with_keyed_seeds` surfaces the key values in the plan's dirty-set
   report and on the propagated result, and the consumer key scope is the downstream's derived
   `KeyScope`.

## Tasks

1. Red: add tests 1–5 against the intended API; watch them fail to compile/assert.
2. Add `KeyValues` + `KeyedDirt::values` in `propagate.rs`; update the existing keyed
   assertions in `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs`.
3. Add `Edge::consumer_key_scope` + `with_consumer_key_scope`.
4. Add `propagate_with_keys`; make `propagate` a delegating wrapper; implement the projection
   rule as a small pure function (`project_key_values`) with its own doc comment citing
   §"Composition rules".
5. Fix the keyed-only-node visit so keyed dirt composes across hops.
6. Runtime: `plan_since_upstream_with_keyed_seeds` (mirroring the observed-delta wrapper's
   shape), fill `consumer_key_scope` from the derived `PlanCell::key_scope` in the existing
   per-model pass, and surface resolved key values in the dirty-set report.
7. Red→green test 6; narrow the Known Divergences clause per §Spec delta.

## Verification

- `cargo test -p smelt-logical --test keyed_dirt_values --test maintenance_propagation_adjoint`
- `cargo test -p smelt-runtime --test since_upstream_propagation --test typed_edge_graph --test key_addressed_model_edge_lowering`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `bash .claude/scripts/verify-phase.sh`
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches

## Commit message

`feat(incremental): key-valued dirt-sets compose through the propagation graph`
