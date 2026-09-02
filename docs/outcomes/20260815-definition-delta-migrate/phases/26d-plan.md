# Phase 26d — column-group-scoped dirt

## Objective

Close the last clause of success criterion 16: propagated dirt stops being whole-model where the
finer grain is derivable. An inbound edge from a source that is *proven* unable to add or remove
the downstream's rows (the closure-prune proof `grouping.rs` already computes) dirties only the
column groups whose sensitivity names that source, and that scope gates the node's own outbound
edges — a consumer that reads none of the dirty groups is no longer scheduled.

## Spec delta (first)

- `docs/specs/incremental_models.md` §"The graph layer" → add **Column-group-scoped dirt** after
  the keyed-dirt-set subsection: dirt carries an optional column-group scope; the scope is
  admitted **only** when the upstream is a proven row-preserving (closure-pruned) enrichment
  source of the downstream, its group partition is non-degenerate, and it names at least one but
  not all groups. Any other shape — a creation-reaching source, a degenerate collapse, an
  upstream naming no group — is whole-model dirt, unchanged (widen-never-narrow). State that an
  outbound edge carrying no typed components propagates unscoped.
- §Known Divergences, "Locality and diagnostic residues" bullet: delete the
  "column-group-scoped dirt coarsens to whole-partition" clause. If 26a and 26c have landed and
  the only clause left is "the built grain-alignment check validates only the declaration", delete
  the whole bullet — that clause is posture (§"Granularity is declared, not derived"), the same
  call 26c recorded, not a defect.
- Sweep `docs/specs/model_properties.md` §Known Divergences / the `Output-delta shape` and
  per-column-provenance rows where they name the same whole-partition residue.

## Tests (red-green)

`crates/smelt-logical/src/maintenance/grouping.rs` (inline tests):
1. `value_only_sources_names_the_closure_pruned_enrichment` — a proven-`Closed` enrichment join's
   source is reported; the driving source is not.
2. `value_only_sources_is_empty_under_a_degenerate_collapse` — fail-closed.
3. `dirt_scope_narrows_to_the_sensitive_groups` — a value-only upstream naming one of two groups.
4. `dirt_scope_is_whole_for_a_creation_reaching_source` — the driving source → `None`.
5. `dirt_scope_is_whole_when_every_group_is_sensitive` — no narrowing worth carrying.
6. `dirt_scope_is_whole_when_the_upstream_names_no_group` — fail-closed, not "no dirt".

`crates/smelt-logical/src/maintenance/propagate.rs` (inline tests):
7. `scoped_dirt_records_its_groups_on_the_node_and_edge` — `dirty_groups`/`per_edge_groups`
   populated; `dirty`/`per_edge` intervals byte-identical to today.
8. `a_consumer_reading_only_unscoped_groups_is_not_dirtied` — the narrowing's payoff: outbound
   edge whose components name only non-dirty groups contributes nothing.
9. `a_consumer_reading_a_dirty_group_is_dirtied_as_before`.
10. `an_untyped_outbound_edge_propagates_unscoped` — empty `components` ⇒ no gating.
11. `whole_model_dirt_from_a_second_edge_defeats_the_scope` — a node dirtied by one scoped and one
    unscoped edge is whole-model dirty; the scope does not survive the merge.
12. `required_inputs_is_unchanged_by_scoping` — the backward direction ignores group scope.
13. `existing_day_graphs_are_unchanged` — regression anchor over an existing scenario.

`crates/smelt-logical/tests/maintenance_propagation_adjoint.rs`:
14. `adjointness_holds_with_group_scoped_dirt` — `forward(backward(P)) ⊇ P` still holds.

`crates/smelt-runtime/tests/since_upstream_propagation.rs`:
15. `an_enrichment_only_delta_does_not_schedule_an_unaffected_consumer` — end-to-end through
    `build_forward_graph` + `plan_since_upstream`: the consumer disappears from `runs`.
16. `the_plan_report_names_the_column_group_scope` — the per-edge report line renders the groups.

## Tasks

1. Write the spec delta above.
2. `grouping.rs`: surface the proof already computed by `closure_pruned_source` —
   `GroupingResult` gains `value_only_sources: BTreeSet<String>` (sources proven row-preserving at
   the model's top-level scope, so their deltas revise values and never change membership),
   populated where `scan_scope_membership` prunes today. Empty under any degenerate collapse.
3. `grouping.rs`: new pure `pub fn dirt_scope(upstream: &str, result: &GroupingResult) ->
   Option<Vec<String>>` implementing the admission in the spec delta (`None` = whole model), group
   names via `ColumnGroup::name()` so they match `EdgeComponent::group` exactly.
4. `propagate.rs`: `Edge` gains `pub dirtied_groups: Option<Vec<String>>` plus
   `with_dirtied_groups`; `from_clamp` sets `None`. Mechanically default the field at every
   existing `Edge { .. }` literal (~44 sites, mostly tests) — `cargo build` names them.
5. `propagate.rs`: `Propagation` gains `per_edge_groups: BTreeMap<(String, String), Vec<String>>`
   and `dirty_groups: BTreeMap<String, Vec<String>>` (absent = whole-model, so today's consumers
   read unchanged). In the forward walk, merge inbound scopes per node — any unscoped inbound edge
   makes the node whole-model — and gate each outbound edge: skip it when the node's scope is
   `Some(S)` and the edge has non-empty `components` none of whose `group` is in `S`.
6. `smelt-runtime/src/propagation.rs`: extend `consumer_facts_cache` to carry the downstream's
   `GroupingResult`, and set `dirtied_groups` via `dirt_scope(&upstream, ..)` at each `Edge` build.
7. Render the scope on the per-edge report line (`  {downstream} <- {upstream}: {iv} [groups: …]`)
   when present; leave the unscoped line byte-identical.
8. Update `propagate.rs`'s module-doc "Known boundaries" block: the whole-partition-dirt boundary
   is replaced by a statement of the admission and its fail-closed defaults.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib maintenance::grouping --lib maintenance::propagate --test maintenance_propagation_adjoint --test maintenance_tracer_propagation`
- `cargo test -p smelt-runtime --test since_upstream_propagation --test typed_edge_graph --test tracer_propagation`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-cli --features duckdb --test e2e since_upstream`
- `cargo test --workspace` (phase 25: a cross-cutting struct change breaks tests outside the
  phase's own file list — sweep before declaring green)

## Commit message

`feat(maintenance): scope propagated dirt to the column groups a value-only source can reach`
