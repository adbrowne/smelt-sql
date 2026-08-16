# Phase 5 plan — propagated key restrictions reach the key-addressed cell

## Objective

Give the propagated keyed dirt-set a way to reach the cell that actually runs: a request-level
keyed-restriction channel on `ExecuteRequest`, **unioned** into the affected-key relation the
key-addressed repair cell reads, with `--since-upstream` converting its `SinceUpstreamPlan::
keyed_dirty` into that channel. Advances success criterion 2 (value-level discovery feeds the
scheduler, not only the run-time mechanism) and completes the dispatch half of criterion 1: today
the sidecar diff is the *only* source of the repaired key set, so a key the graph layer knows is
dirty is silently not repaired unless the upstream's own fingerprint happens to have changed.

## Spec delta

`docs/specs/incremental_models.md`, §"Dispatch — from propagated components to run units" — add
one paragraph, **"Restrictions compose by union"**, after "Widen-never-narrow at dispatch":

- A key-addressed run unit's read restriction is the **union** of (a) the keyed component's
  propagated key values and (b) the values the cell's own affected-key discovery resolves
  (§"Upstream model edges" — the group-grain fingerprint sidecar diff). Never an intersection.
- Rationale, stated normatively in one sentence: the sidecar refresh commits in the same
  transaction as the write, so narrowing the repaired set would advance the comparandum past keys
  that were never consumed — wrong-and-quiet.
- A propagated component whose values are unresolved (§"Unresolved seeds") contributes **no**
  keys to the union and never narrows it; it widens at dispatch by the rule already stated.

No other spec file changes. Do not touch the Known Divergences bullets — rows 6–8 own them.

## Tests

Red-green, in this order.

1. `smelt-runtime/tests/key_addressed_model_edge_lowering.rs::propagated_restriction_key_is_repaired_when_sidecar_reports_no_change`
   — e2e, real DuckDB: downstream row is stale for key `k2` whose upstream fingerprint is
   unchanged; a request carrying `k2` as a keyed restriction repairs it. The load-bearing test.
2. `…::propagated_restriction_alone_dispatches_when_sidecar_diff_is_empty` — sidecar reports zero
   changed keys but the restriction is non-empty: the cell dispatches (not the `Ok(None)` no-op).
3. `…::empty_restriction_leaves_discovery_unchanged` — regression pin: with an empty restriction
   the resolved key set and emitted affected-keys SELECT are identical to today's.
4. `…::restriction_unions_with_sidecar_keys_never_intersects` — restriction `{k2}`, sidecar
   `{k1}` → resolved set is `{k1, k2}` (sorted, deduped), asserted on the returned key list.
5. `smelt-runtime/tests/since_upstream_propagation.rs::keyed_restrictions_from_plan_carries_resolved_values`
   — a plan built via `plan_since_upstream_with_keyed_seeds` converts to a
   `model → [KeyedRestriction{upstream, keys, values}]` map.
6. `…::keyed_restrictions_from_plan_drops_unresolved_values` — a `KeyValues::Unresolved` entry
   yields no map entry (contributes nothing to the union; never narrows).

## Tasks

1. Land the §Spec delta paragraph above (spec-first, before code).
2. Add plain-data `KeyedRestriction { upstream: String, keys: Vec<String>, values: Vec<String> }`
   to `smelt-runtime/src/types.rs` (mirrors `CellTechniqueOverride`'s precedent — a serde wire
   type, not a re-export of `smelt_logical`'s `KeyedDirt`).
3. Add `#[serde(default)] pub keyed_restrictions: BTreeMap<String, Vec<KeyedRestriction>>` to
   `ExecuteRequest`, keyed by consumer model name; update every `ExecuteRequest { … }` literal
   (~72 sites; `rg -n 'ExecuteRequest \{' -g '*.rs' crates`) to `BTreeMap::new()`.
4. `smelt-runtime/src/propagation.rs`: pure `pub fn keyed_restrictions_from_plan(&SinceUpstreamPlan)
   -> BTreeMap<String, Vec<KeyedRestriction>>` — `KeyValues::Resolved` only, sorted/deduped values.
5. `smelt-cli/src/commands/run.rs::run_since_upstream`: populate each per-model `ExecuteRequest`'s
   `keyed_restrictions` from that function (pass the whole map; `execute_project` selects by model).
6. `smelt-runtime/src/maintenance_driver.rs`: thread `restriction_keys: &[String]` into
   `resolve_key_addressed_affected_keys` (union + sort + dedup with the sidecar's `changed_keys`
   *before* calling `emit_key_addressed_affected_keys_select` — set arithmetic on inputs, the
   emitter stays the single owner of the statement) and into
   `execute_key_addressed_model_edge_cell`; the empty-set no-op guard now tests the **union**.
7. `smelt-runtime/src/execute.rs`: at both dispatch sites, look up
   `request.keyed_restrictions.get(model_name)` and select the entry whose `upstream == edge_name`;
   pass its `values` (empty slice when absent) through `dispatch_key_addressed_model_edge`.
8. Doc comments at each new seam citing the spec paragraph; note the union rule inline at the
   place the union is computed.

## Verification

- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering --test
  since_upstream_propagation`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `bash .claude/scripts/verify-phase.sh`
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches.

## Commit message

`feat(incremental): propagated key restrictions union into the key-addressed cell`
