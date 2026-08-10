# Phase 5 plan — Keyed dirt-set propagation for admitted shapes

## Objective

Make `propagate`/`required_inputs` **act** on `Edge.components` instead of treating them as
advisory: an edge touching a keyed-grain node whose component vector carries an
`Addressing::Keyed` component propagates a **keyed dirt-set** rather than being refused, and the
`MaintenanceGraphUnsupportedNode` refusal narrows to the case where the node's shape degraded to
`General` (or no verdict exists at all — fail closed), naming the degrading operator. Advances
success criterion 4, and unblocks criterion 3's end-to-end chain (phase 6) by removing the
blanket refusal that stops a keyed upstream from reaching its consumer.

## Spec delta

Behaviour is already normative (`incremental_models.md` §"The graph layer" → "Keyed dirt-sets and
the narrowed refusal", landed phase 1). Two **status** edits only, made by the implement step:

- `docs/specs/model_properties.md` §Known Divergences, bullet "Output-delta shape is derived and
  typed onto propagation edges, but not yet acted on by dirt propagation" — rewrite: `propagate`
  and `required_inputs` now read `Edge.components`; keyed dirt-sets propagate for an admitted
  `KeyedUpsert` component and the refusal is narrowed to `General`/absent verdicts. State the
  remaining gap honestly: the keyed dirt-set is the **symbolic** key-addressed channel (key
  columns + provenance), not a materialised key-value set — value-level affected-key discovery
  stays with the run-time mechanism (§"Affected-key discovery"). Also update the §Properties
  table row for "Output-delta shape" (`consumed by edge typing, not yet by dirt propagation`).
- `docs/specs/incremental_models.md` §Known Divergences, bullet "Edge typing is specified but the
  propagation layer still carries day intervals only" — narrow it to the same remaining gap.

## Tests

Red-green, in `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs` (existing home of
the keyed refusal tests) unless named otherwise:

1. `keyed_upstream_with_keyed_component_is_not_refused` — an edge whose upstream is
   `PartitionGrain::Keyed` and whose components carry `Addressing::Keyed{keys}` propagates instead
   of erroring.
2. `keyed_node_with_general_component_still_refuses_naming_the_operator` — the same edge with a
   `WholeModel{degraded_by}` component refuses `MaintenanceGraphUnsupportedNode`, and the message
   contains `degraded_by`'s text.
3. `keyed_node_without_components_fails_closed` — no components ⇒ the existing refusal survives
   verbatim (pins fail-closed; the existing bare-keyed tests at ll. 303–320 must keep passing).
4. `keyed_edge_propagates_a_keyed_dirt_set_not_intervals` — the downstream gets an entry in the
   new keyed channel keyed by `(downstream, upstream)` with the component's key columns, and
   **no** interval reflected through that edge into `per_edge`.
5. `keyed_dirt_into_a_clocked_consumer_widens_to_whole` — a keyed-addressed edge whose downstream
   is clocked yields `DayInterval::WHOLE` interval dirt for that consumer (widen-never-narrow:
   never nothing).
6. `required_inputs_over_a_keyed_ancestor_requires_the_whole_table` — backward resolution through
   an admitted keyed edge yields `WHOLE` for the keyed ancestor and keeps the target's own
   interval math unchanged; `build_order` still includes it.
7. `adjoint_property_holds_with_keyed_edges_present` — `forward(backward(P)) ⊇ P` still holds on a
   graph mixing window- and keyed-addressed edges (extend the existing adjoint property test).
8. `crates/smelt-runtime/tests/typed_edge_graph.rs::keyed_upstream_model_propagates_on_the_real_graph`
   — a `GROUP BY`-over-append-only keyed model feeding a downstream builds and propagates through
   `build_forward_graph` + `propagate` without a refusal.
9. `crates/smelt-runtime/tests/since_upstream_propagation.rs::bare_keyed_origin_refusal_narrows_to_general`
   — `refuse_bare_keyed_origins` consults the model's derived output-delta verdict: a
   `KeyedUpsert` origin is admitted, a `General` one still bails with the narrowed message.

## Tasks

1. Add the keyed channel to `maintenance::propagate`: `KeyedDirt { keys: Vec<String>, from: String }`
   plus `Propagation::per_edge_keys: BTreeMap<(String,String), Vec<KeyedDirt>>` and
   `keyed_dirty: BTreeMap<String, Vec<KeyedDirt>>` (additive — existing interval fields and their
   consumers unchanged).
2. Replace `refuse_keyed_nodes` with `classify_keyed_edges`: per edge touching a `Keyed`-grain
   endpoint, admit iff some component has `Addressing::Keyed`; otherwise refuse with the
   `MaintenanceGraphUnsupportedNode` message naming `WholeModel{degraded_by}` (or the existing
   bare-keyed text when the vector is empty).
3. In `propagate`, route an admitted keyed edge through the keyed channel: record `KeyedDirt` on
   `per_edge_keys`/`keyed_dirty`, and emit `DayInterval::WHOLE` interval dirt for the downstream
   when the downstream is *not* keyed (so a clocked consumer of a keyed node still runs).
4. In `required_inputs`, an admitted keyed edge requires `DayInterval::WHOLE` upstream (no clamp
   arithmetic on a non-existent axis) and participates in `build_order` normally.
5. Narrow `smelt-runtime::propagation::refuse_bare_keyed_origins` the same way — consult the
   workspace output-delta verdicts already derived in `build_forward_graph`
   (`workspace_output_delta_verdicts`), refusing only on `General`/absent.
6. Update the two Known-Divergences bullets + the property-table row per §Spec delta; keep
   `output_delta_spec.rs`'s section assertions passing (add one for the narrowed refusal wording
   if the existing assertion no longer pins it).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_propagation_adjoint --test typed_edge_spec --test output_delta_spec --test output_delta_workspace --test walk_coverage`
- `cargo test -p smelt-runtime --test typed_edge_graph --test since_upstream_propagation --test tracer_propagation --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`

## Commit message

`feat(logical): keyed dirt-set propagation for admitted shapes; refusal narrowed to General`
