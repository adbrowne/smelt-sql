# Phase 4 plan — Consumer-side fold over an upstream keyed-upsert delta (model-edge change-feed)

## Objective

Make the model-reference leaf real: a consuming model's walk resolves an upstream **model's**
already-derived output-delta verdict per column instead of failing closed to `General`, folded
across the workspace in dependency order. Then wire `type_edge` into the real graph builder so
`Edge.components` is non-empty in production, carrying `Keyed` addressing for a keyed-upsert
upstream. Advances success criteria 1 (the model-reference transfer row), 2 (typed edges on real
edges, not just in tests) and unblocks 4/5 (keyed dirt-sets need a real component to act on).

## Spec delta

`docs/specs/model_properties.md` §Known Divergences, the "Output-delta shape is derived and typed
onto propagation edges…" bullet: state that verdicts now fold **across model references** in the
real workspace graph (a consuming model reads its upstream's verdict; edges in
`build_forward_graph` carry typed components), while keeping the accurate remaining gap — neither
`propagate` nor `required_inputs` reads `Edge.components`, so interval math is still the only dirt
currency. No §Surface maturity-row change (still `partial`). No user-facing docs-site change
(`smelt explain` rendering stays phase 7).

## Tests

- `output_delta::tests::model_reference_leaf_resolves_per_column_from_upstream_facts` — a
  `model_verdicts` entry supplies per-output-column shapes; two differently-shaped upstream
  columns stay distinct through a pass-through consumer (no scalar meet across the upstream).
- `output_delta::tests::model_reference_column_absent_from_upstream_is_general` — a referenced
  column the upstream verdict does not carry is `General`, naming model + column.
- `crates/smelt-logical/tests/output_delta_workspace.rs::chain_folds_verdicts_in_dependency_order`
  — source(append_only) → A (`GROUP BY key`) → B (projection): B's verdict is `KeyedUpsert`, not
  `General`.
- `…::passthrough_model_preserves_append_only_window` — source(append_only) → A (`SELECT … WHERE`)
  → B: B stays `AppendOnlyWindow` on the same axis.
- `…::unknown_or_cyclic_model_ref_is_general_and_terminates` — a ref to a model not in the input
  set, and a two-model cycle, both yield `General` and the fold returns (bounded passes, no hang).
- `crates/smelt-runtime/tests/typed_edge_graph.rs::forward_graph_edge_from_model_upstream_carries_components`
  — `build_forward_graph` over a two-model fixture (keyed upstream, consumer) produces an edge
  whose `components` is non-empty with `Addressing::Keyed`.
- `…::forward_graph_edge_from_window_source_carries_window_component` — a `sources.*` upstream
  edge carries a `Window{axis}` component via `SourceFacts::from_source_info`.
- `…::component_population_does_not_change_interval_dirt` — `propagate` over the typed graph
  returns the same dirt as over the same graph with `components` cleared (advisory invariant).
- `crates/smelt-logical/tests/output_delta_spec.rs::known_divergence_states_cross_model_fold` —
  the spec bullet names the cross-model fold and the remaining propagation gap.

## Tasks

1. Expose the walk's per-output-column result: `analysis::output_delta::derive_output_delta_facts(
   sql, ctx, sources, model_verdicts) -> Option<OutputDeltaFacts>`; re-express
   `derive_output_delta` in terms of it (no behaviour change for existing callers).
2. Change `OutputDeltaTransfer::model_verdicts` to `&BTreeMap<String, OutputDeltaFacts>` (bare
   model name, lowercased) and resolve a model-reference leaf **per column** against that facts
   vector; an absent model or absent column ⇒ `General` naming what was missing.
3. Confirm the leaf name the walk produces for a real `smelt.ref('a.b')` model reference and make
   `seed_for_leaf_name` key on it (the existing `models.` prefix branch may need to accept the
   canonical dotted address form instead); pin it with one of the workspace tests above.
4. Add `analysis::output_delta::derive_workspace_output_deltas(inputs) -> BTreeMap<String,
   OutputDeltaFacts>` — one input record per model (address, sql, ctx, sources, skeleton columns,
   referenced model addresses), folded in dependency order via a bounded fixed-point pass
   (`inputs.len() + 1`, mirroring `derive_clamp_and_locality`'s bound) so a cycle terminates
   fail-closed at `General` rather than hanging.
5. In `smelt-runtime::propagation`, build the per-model input records from the same `ModelFile`
   /`SourceInfo` data `derive_clamp_and_locality_pass` already walks (`strip_frontmatter`, refs,
   `SourceFacts::from_source_info` for declared sources) and call the workspace fold once.
6. In `build_forward_graph`, replace `components: Vec::new()` with `type_edge(&upstream,
   upstream_group_verdicts, consumer_read_columns, consumer_groups)`, deriving the consumer's read
   columns from its resolved column references and its groups from the same
   `derive_column_groups`-backed path `derive_output_delta` uses; an upstream with no derived
   verdict contributes no component (never a fabricated one).
7. Apply the spec-delta bullet edit and update `output_delta_spec.rs`.
8. Record in `phases/04-summary.md` what phase 5 needs: which components exist in real graphs and
   where `propagate`/`required_inputs` would read them.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test output_delta_workspace --test output_delta_spec --test
  typed_edge_spec --test walk_coverage --test maintenance_propagation_adjoint`
- `cargo test -p smelt-logical output_delta` (lib unit tests)
- `cargo test -p smelt-runtime --test typed_edge_graph --test tracer_propagation --test
  statement_parity --test execute_parity`

## Commit message

`feat(logical): cross-model output-delta fold and typed components on the real propagation graph`
