# Phase 3 plan — Edge typing in the propagation layer; adjoint property preserved

## Objective

Give a propagation `Edge` a **vector of typed components** — `(delta shape × addressing ×
column set)` — derived from the upstream's `derive_output_delta` verdicts projected through
the consumer's own per-column sensitivity, and prove that today's day-interval forward /
backward maps are exactly the window-addressed case with `forward(backward(P)) ⊇ P`
unchanged. Advances success criteria 2 (typed edges, adjoint preserved) and unblocks 3/4/5
(the consumer fold and keyed dirt-sets read these components).

## Spec delta

`docs/specs/model_properties.md` §"Output-delta shape" — §Surface maturity row for the
output-delta verdict moves from `partial (derived; not yet consumed by edge typing)` to
`partial (derived; consumed by edge typing, not yet by dirt propagation)`. No behaviour
delta in `incremental_models.md`: §"The graph layer" §"Typed edges" (landed phase 1) is the
normative text this phase implements verbatim — components per column group the consumer
reads, day-interval dirt is the `AppendOnlyWindow`-under-window-addressing component, and
a component that cannot project degrades to whole-model dirt, never to nothing.

## Tests

Red-green, all in `smelt-logical` unless noted.

New `crates/smelt-logical/src/maintenance/edge_type.rs` unit tests:
1. `window_component_from_append_only_upstream` — an `AppendOnlyWindow{axis}` upstream group
   the consumer reads yields one component with `Addressing::Window { axis }`.
2. `keyed_component_from_keyed_upsert_upstream` — `KeyedUpsert{k}` yields
   `Addressing::Keyed { keys: k }`, column set = that group's columns.
3. `general_component_degrades_to_whole_model` — `General{reason}` yields
   `Addressing::WholeModel` carrying `degraded_by = reason`; the component is **present**,
   not dropped (widen-never-narrow).
4. `component_per_group_not_per_model` — a two-group upstream (one append-only, one general)
   read by the consumer yields two components; the general one does not degrade the other.
5. `groups_the_consumer_does_not_read_are_omitted` — an upstream group whose columns are
   outside the consumer's read/sensitivity set produces no component.
6. `unprojectable_axis_degrades_to_whole_model` — an `AppendOnlyWindow` whose axis column is
   not carried into the consumer's group degrades to `WholeModel`, naming the axis.
7. `components_are_deterministically_ordered` — stable ordering by group name.
8. `source_facts_from_source_info_seeds_leaf` (in `analysis/output_delta.rs`) — the new
   `SourceFacts::from_source_info` adapter maps declared `append_only`+clock →
   `AppendOnlyWindow`, `change_feed`+`delta_identity` → `KeyedUpsert`, undeclared → `General`
   (mirrors `input_delta::SourceShape::from_source_info`; fail-closed).

In `crates/smelt-logical/src/maintenance/propagate.rs`:
9. `typed_edge_forward_matches_untyped_for_window_components` — an `Edge` carrying only
   window-addressed components propagates byte-identical `Propagation` to today's untyped edge.
10. `adjoint_holds_with_typed_components` — property-style over a small fixed DAG:
    `forward(backward(P)) ⊇ P` for edges carrying window components (the spec's adjoint claim,
    re-asserted with the new field populated).
11. `whole_model_component_does_not_narrow_dirt` — an edge with a `WholeModel` component
    still yields at least the interval dirt the untyped edge would (never narrower).

New spec-table test `crates/smelt-logical/tests/typed_edge_spec.rs`:
12. `typed_edges_section_names_the_three_component_parts` — §"Typed edges" names delta shape,
    addressing and column set, and states the widen-never-narrow degrade.
13. `output_delta_surface_row_matches_phase_maturity` (extend existing `output_delta_spec.rs`
    assertion) — the updated maturity string.

## Tasks

1. Add `SourceFacts::from_source_info` to `analysis/output_delta.rs`, mirroring
   `input_delta::SourceShape::from_source_info` (test 8).
2. New `crates/smelt-logical/src/maintenance/edge_type.rs`: `Addressing`
   (`Window { axis } | Keyed { keys } | WholeModel { degraded_by }`) and `EdgeComponent`
   (`shape: OutputDelta`, `addressing: Addressing`, `columns: Vec<String>`,
   `group: String`) — pure data, no I/O.
3. Implement `type_edge(upstream: &str, upstream_verdicts: &[(ColumnGroup, OutputDelta)],
   consumer_read_columns: &BTreeSet<String>, consumer_groups: &[ColumnGroup]) ->
   Vec<EdgeComponent>`: filter to groups the consumer reads, map shape → addressing,
   degrade unprojectable components to `WholeModel` naming the cause, sort deterministically
   (tests 1–7).
4. Add `pub components: Vec<EdgeComponent>` to `propagate::Edge`; `Edge::from_clamp` leaves it
   empty (today's behaviour = the window-addressed case), add `Edge::with_components`.
   Update the handful of struct-literal `Edge { .. }` construction sites (`smelt-runtime/src/
   propagation.rs:241`, `smelt-runtime/tests/tracer_propagation.rs:793`).
5. Assert in `propagate`/`required_inputs` that components are advisory this phase — no
   change to interval math — and cover it with tests 9–11.
6. Register the module in `maintenance/mod.rs`; re-export `Addressing`/`EdgeComponent`/
   `type_edge` from `lib.rs`.
7. Apply the spec delta (maturity row) and write tests 12–13.
8. Write `phases/03-summary.md` (shipped / decisions / for-the-next-planner, flagging what
   phase 4's consumer fold and phase 5's keyed dirt-sets must read from these components).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test typed_edge_spec --test output_delta_spec --test walk_coverage`
- `cargo test -p smelt-logical edge_type`
- `cargo test -p smelt-runtime --test tracer_propagation` (edge construction sites still build
  and propagate identically)

## Commit message

`feat(logical): typed propagation-edge components derived from upstream output-delta verdicts`
