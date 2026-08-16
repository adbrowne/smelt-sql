# Phase 3 summary — Edge typing in the propagation layer; adjoint property preserved

**Shipped:**
- `crates/smelt-logical/src/maintenance/edge_type.rs` (new): `Addressing` (`Window{axis}` |
  `Keyed{keys}` | `WholeModel{degraded_by}`), `EdgeComponent` (`group`, `shape: OutputDelta`,
  `addressing`, `columns`), and `type_edge(upstream, upstream_verdicts, consumer_read_columns,
  consumer_groups) -> Vec<EdgeComponent>`: one component per upstream column group the consumer
  reads (a group outside the read set is omitted, not degraded), `AppendOnlyWindow` projects to
  `Window` only when its axis column is carried into one of the consumer's own derived column
  groups (else `WholeModel`, naming the axis), `KeyedUpsert` projects to `Keyed` unconditionally,
  `General` projects to `WholeModel` carrying its reason verbatim. Sorted by group name. 7 unit
  tests.
- `crates/smelt-logical/src/analysis/output_delta.rs`: `SourceFacts::from_source_info(name,
  &SourceInfo)`, mirroring `input_delta::SourceShape::from_source_info`'s fail-closed pattern
  (axis from `timeseries:`, mutation profile + delta identity from the declared
  `mutation_profile:` block). 1 unit test covering all three profile outcomes.
- `crates/smelt-logical/src/maintenance/propagate.rs`: `Edge` gained `pub components:
  Vec<EdgeComponent>` (advisory this phase — `propagate`/`required_inputs` do not read it) and
  `Edge::with_components`; `Edge::from_clamp` leaves it empty. Every struct-literal `Edge { .. }`
  site updated (`propagate.rs` internal test helpers ×2, `smelt-runtime/src/propagation.rs`,
  `smelt-runtime/tests/tracer_propagation.rs`, `smelt-logical/tests/
  maintenance_tracer_propagation.rs` ×2, `smelt-logical/tests/maintenance_propagation_adjoint.rs`)
  — all found via the compiler's own `E0063 missing field` diagnostics, not just the two the plan
  named. 3 new tests (`typed_edge_advisory_tests`): typed vs untyped forward propagation is
  byte-identical for a window component, the adjoint property `forward(backward(P)) ⊇ P` holds
  with typed components attached over a two-hop DAG, and a `WholeModel` component does not narrow
  interval dirt below the untyped edge's own result.
- Module registered in `maintenance/mod.rs`; `Addressing`/`EdgeComponent`/`type_edge` re-exported
  from `lib.rs` alongside the existing `output_delta` re-exports.
- `docs/specs/model_properties.md`: §Surface maturity row for "Output-delta shape" now `partial
  (derived; consumed by edge typing, not yet by dirt propagation)`; the stale Known Divergences
  bullet (left over from phase 1, claiming "no walk transfer function computes the verdict yet"
  — already false after phase 2) rewritten to state the current, accurate gap.
- `crates/smelt-logical/tests/output_delta_spec.rs`: `surface_row_exists_for_output_delta` updated
  to the new maturity string.
- `crates/smelt-logical/tests/typed_edge_spec.rs` (new): asserts §"The graph layer" → "Typed
  edges" names delta shape, addressing, and column set, and states the widen-never-narrow degrade
  (component present, never dropped).

**Decisions:**
- The window-axis "carried into the consumer" check reads the consumer's own **derived column
  groups** (`consumer_groups`), not just `consumer_read_columns` — the two params serve different
  questions: `consumer_read_columns` decides *which upstream groups are read at all* (filters out
  untouched groups), `consumer_groups` decides *whether the axis specifically survives the
  consumer's own projection* (a consumer can read `amount` from an append-only upstream group
  without projecting that group's `event_date` axis into its own output, in which case a further
  downstream edge could no longer address by that axis — degrade to `WholeModel` rather than
  claim an addressing the consumer's own schema doesn't carry).
- `KeyedUpsert` needs no carriage check the way `AppendOnlyWindow` does: a key set is addressable
  as long as the keyed columns are read at all (already gated by the group-is-read filter above),
  unlike a window axis which specifically must survive as its own carried column.
- Components are typed **per column group, never meet-folded across groups** — mirrors
  `model_properties.md`'s own per-column-group scoping for `OutputDelta` itself, so a mixed
  upstream (one append-only group, one general group) yields two independent components instead
  of collapsing the edge to one worst-case shape.

**For the next planner:**
- Phase 4 (consumer-side fold) needs: a real caller of `type_edge` wiring `derive_output_delta`'s
  upstream verdicts against a downstream model's actual read columns/derived groups — nothing
  calls `type_edge` from `smelt-runtime::propagation::build_forward_graph` yet, so `Edge.components`
  is empty in every real graph today (test-only so far). The `SourceFacts::from_source_info`
  adapter this phase adds is the piece phase 2's summary flagged as missing; it's ready for that
  wiring, still uncalled from production code.
- The `model_verdicts` cross-model fold on `OutputDeltaTransfer` (phase 2's flagged gap) is still
  unwired — phase 4's consumer fold is still the intended place to thread a real
  upstream-model-verdict map through `derive_output_delta`.
- Phase 5 (keyed dirt-sets) and dirt propagation acting on `Edge.components` at all are both still
  open — this phase intentionally keeps `propagate`/`required_inputs` reading only the existing
  interval fields, per the plan's "advisory this phase" scope.
- No phase-table reshape — phase 3's scope (component derivation + edge field, no
  propagation/graph-builder wiring) matched the plan exactly.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test typed_edge_spec --test output_delta_spec --test
  walk_coverage` — 1/1 + 6/6 + 4/4 passed.
- `cargo test -p smelt-logical edge_type` (lib) — 7/7 unit tests passed.
- `cargo test -p smelt-runtime --test tracer_propagation` — 6/6 passed (edge construction sites
  build and propagate identically with the new field).
