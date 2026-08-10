# Phase 6 plan — Key-addressed model-edge cells (plan derivation)

## Objective

Make a maintained upstream whose derived output-delta shape is `KeyedUpsert` contribute a
**key-addressed** plan cell to its consumer: no clock column required on the upstream, and a
keyed-grain consumer (no output partition axis) gets a cell instead of nothing. This unblocks
success criterion 3 (a keyed upstream → consumer chain that folds the upstream's upsert delta)
and completes criterion 2's "addressing" leg on the *plan* side; lowering is phase 7, the
conformance chain is phase 8. Scope is plan derivation only — no statement emission, no driver.

## Spec delta (spec-first — the implementer makes this edit first)

`docs/specs/incremental_models.md`:

- §"Upstream model edges" — add: an upstream maintained model whose derived output-delta shape is
  `KeyedUpsert` contributes a **key-addressed** edge. Its cell's read restriction is the affected
  key set the upstream's delta names (`Technique::PerGroupRecompute`), not a partition interval;
  no `timeseries:` clock is required on the upstream, and a keyed-grain consumer receives such a
  cell. `ReachNotDerivable` narrows to a clockless upstream whose shape is **not**
  keyed-addressed. Fail-closed leg stated explicitly: if the consumer's SQL does not carry the
  upstream's key columns, the edge is refused (no silent whole-table cell).
- §"Known Divergences" — replace/narrow the "keyed fold, deferred" note: the cell now derives;
  what remains deferred is its lowering (phase 7's scope) — state it as behaviour ("a
  key-addressed model-edge cell is derived and rendered but not yet executed"), no phase words.

## Tests (red → green)

`crates/smelt-logical/tests/keyed_model_edge.rs` (new):

1. `clockless_keyed_upstream_yields_a_key_addressed_cell` — partition-grain consumer of a
   `KeyedUpsert`, clockless upstream: one `PlanCell` with `technique: PerGroupRecompute` and a
   `key_scope` naming the upstream's key columns; zero `ReachNotDerivable` refusals.
2. `keyed_consumer_of_keyed_upstream_yields_a_cell` — `output_partition_col: None`: the same cell
   derives (the early return no longer swallows a keyed consumer).
3. `clockless_non_keyed_upstream_still_refuses` — an `AppendOnlyWindow`/`General`-shaped clockless
   upstream keeps the verbatim `ReachNotDerivable` refusal (narrowing, not removal).
4. `key_addressed_cell_claims_no_interval_scan` — the cell's `scans` is empty and
   `partition_local` is the honest `No { .. }` naming key addressing; it never reports a partition
   clamp it does not have.
5. `consumer_not_carrying_upstream_keys_is_refused` — consumer SQL that projects none of the
   upstream's key columns produces a refusal naming the missing keys, never a cell.

`crates/smelt-runtime/tests/typed_edge_graph.rs`:

6. `clockless_keyed_upstream_is_walkable_on_the_real_graph` — the phase-5 fixture with the
   synthetic `timeseries:` block on the keyed model **removed**: `build_forward_graph` still
   yields an edge carrying an `Addressing::Keyed` component and `propagate` admits it.
   (Phase 5's existing test stays; this is the natural-fixture sibling.)

## Tasks

1. Make the spec edit above.
2. `smelt-logical/src/maintenance/derive.rs`: `ModelEdge` gains additive edge shape facts —
   the upstream's `OutputDelta` shape plus its key columns (default = today's behaviour).
3. `smelt-logical/src/maintenance/mod.rs`: `PlanCell` gains additive `key_scope: Option<KeyScope>`
   (`{ keys: Vec<String>, from: String }`); every existing constructor sets `None`.
4. `smelt-logical/src/maintenance/repair.rs`: add `admit_key_addressed_recompute` — a sibling of
   `admit_per_group_recompute` whose slice is the key set rather than a `ScanClamp`; refuses when
   the consumer SQL does not carry the upstream's key columns (reuse `derive_affected_keys`'s
   context and refusal vocabulary).
5. `append_model_edge_cells`: branch on the keyed edge **before** the `clock_col` refusal and
   **before** the `output_partition_col` early return; on admission push the `PerGroupRecompute`
   cell (empty `scans`, honest `partition_local`, populated `key_scope`), on refusal push the
   named refusal.
6. `smelt-runtime/src/propagation.rs`: populate the new `ModelEdge` facts from
   `derive_workspace_output_deltas`' verdicts (the map phase 4 already builds) instead of leaving
   them defaulted.
7. Fix any coverage-matrix / plan-conformance fixtures that enumerate cells or refusals
   (`maintenance_coverage_matrix.rs`, `maintenance_plan_refusals.rs`) — do not weaken an
   assertion to accommodate the new cell; update its expected set explicitly.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test keyed_model_edge --test model_edge_delta_restriction --test repair_cell --test maintenance_plan_refusals --test maintenance_coverage_matrix --test maintenance_propagation_adjoint --test walk_coverage`
- `cargo test -p smelt-runtime --test typed_edge_graph --test since_upstream_propagation --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance --test explain_maintenance`

## Commit message

`feat(logical): key-addressed model-edge cells for keyed-upsert upstreams`
