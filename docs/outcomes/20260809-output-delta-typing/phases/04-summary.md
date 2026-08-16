# Phase 4 summary — Consumer-side fold over an upstream keyed-upsert delta

**Shipped:**
- `crates/smelt-logical/src/analysis/output_delta.rs`: `derive_output_delta_facts` (the walk alone,
  no `ColumnGroup` fold), `derive_output_delta_with_model_verdicts` (the full form
  `derive_output_delta` now wraps), `derive_consumer_column_groups`, `referenced_column_names`,
  `seed_shape_for_source`/`source_output_delta` (free functions, extracted so a caller with only a
  `SourceFacts` in hand can seed a shape without a SQL walk), `ModelDeltaInput` +
  `derive_workspace_output_deltas` (bounded `inputs.len() + 1`-pass fixed-point fold across model
  references, mirroring `smelt-runtime`'s own `derive_clamp_and_locality` convergence argument).
  `OutputDeltaTransfer::model_verdicts` is now `&BTreeMap<String, OutputDeltaFacts>` (per output
  column, not a scalar per model) and resolves a `models.*` leaf per column
  (`seed_for_model_column`); an absent model or column fails closed to `General` naming both.
  `OutputDeltaFacts` gained `PartialEq`. 2 new unit tests.
- `crates/smelt-logical/tests/output_delta_workspace.rs` (new): 3 tests — a chain folds a
  `KeyedUpsert` verdict across a model reference, a pass-through model preserves
  `AppendOnlyWindow`, an unknown/cyclic model ref is `General` and the fold terminates.
- `crates/smelt-runtime/src/propagation.rs`: `build_forward_graph` now calls
  `derive_workspace_output_deltas` once per real graph build, then `type_edge` per edge —
  `Edge.components` is non-empty in production, not just in tests. New helpers:
  `model_output_delta_sources`, `model_skeleton_columns`, `workspace_output_delta_verdicts`,
  `upstream_output_delta_groups` (model upstream folds workspace verdicts; source upstream seeds
  via `source_output_delta`), `consumer_output_delta_facts` (read columns + derived groups).
- `crates/smelt-runtime/tests/typed_edge_graph.rs` (new): 3 tests — a `sources.*` upstream edge
  carries a `Window{axis}` component; a keyed-upsert model upstream (`GROUP BY` over an
  append-only source) feeding a downstream via `smelt.models.*` carries a `Keyed` component on the
  real graph; populating components does not change `propagate`'s interval dirt.
- `docs/specs/model_properties.md` §Known Divergences: the output-delta bullet now states the
  cross-model fold and real `build_forward_graph` caller, keeping the accurate remaining gap
  (`propagate`/`required_inputs` still read only interval fields). Pinned by a new
  `output_delta_spec.rs` test.

**Decisions:**
- `OutputDeltaTransfer::model_verdicts` is per output column (`OutputDeltaFacts`), not a scalar per
  model — a scalar would meet-fold a mixed-shape upstream to its worst group, contradicting the
  2026-08-09 per-column-group decision. Taken in-plan (the plan's own point 2).
- **Fixed a pre-existing addr-resolution bug** in `derive_clamp_and_locality_pass`: a
  `smelt.models.<addr>` ref's own segments carry the literal `models` keyword
  (`SmeltRef::to_path`), but `ModelFile::canonical_path()` never does — the existing `addr =
  segs.join(".")` for a model ref could never match `model_by_addr`, so no model-edge maintenance
  cell had ever been derived through the real graph builder for ANY workspace (only unit-tested
  directly against `append_model_edge_cells`). Generalized `bare_name` to strip a leading `models`
  segment too (safe: no existing call site's segments ever start with `models`) and reused the
  already-computed `bare` instead of recomputing `addr`. This was blocking to this phase's own
  success criterion 3 (model-edge change-feed), not a pre-existing target — fixed in place per the
  outcome loop's "red on the phase's own target" allowance.
- `derive_consumer_column_groups` appends a synthetic `ColumnGroup` covering the model's own
  skeleton columns (e.g. the declared `timeseries.partition_column`): `derive_column_groups`
  deliberately excludes skeleton columns from its payload partition, but `type_edge`'s
  window-axis "carried into the consumer" check needs to find that exact column somewhere in
  `consumer_groups` — without the synthetic group, every window component degraded to
  `WholeModel` even when the axis plainly survives in the consumer's own output.

**For the next planner:**
- Phase 5 (keyed dirt-sets) needs: real `Edge.components` now exist in production graphs
  (`Addressing::Keyed`/`Window`/`WholeModel`) — `propagate`/`required_inputs` are the two call
  sites that still read only `before_days`/`after_days`/interval math, per this phase's own scope.
- `referenced_column_names`'s consumer-read-columns filter is name-only (unqualified), matching
  `type_edge`'s own pre-existing filter contract — a same-named column from two different upstream
  groups is not disambiguated. Not exercised by a failure in this phase's fixtures; flagged as a
  known coarseness, not a bug, should it surface later.
- `derive_workspace_output_deltas` runs over every model in the workspace unconditionally (not
  just `refresh: incremental` ones), since a model-reference leaf may name any model. For a large
  workspace this is `O(models^2)` worst case (bounded passes × full re-walk per model); untested
  at scale — flag if a real workspace shows this as a hot path.
- No phase-table reshape — phase 4's scope (cross-model fold + real graph wiring) matched the plan;
  the addr-bug fix above was the one unplanned item, absorbed in-phase since it blocked criterion 3.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test output_delta_workspace --test output_delta_spec --test
  typed_edge_spec --test walk_coverage --test maintenance_propagation_adjoint` — 3/3 + 7/7 + 1/1 +
  4/4 + 14/14 passed.
- `cargo test -p smelt-logical output_delta` (lib) — 19/19 passed.
- `cargo test -p smelt-runtime --test typed_edge_graph --test tracer_propagation --test
  statement_parity --test execute_parity` — 3/3 + 6/6 + 22/22 + 4/4 passed.
