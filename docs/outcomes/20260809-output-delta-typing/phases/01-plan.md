# Phase 1 — Spec: output-delta types, transfer rules, typed edges, the narrowed keyed refusal

## Objective

Land the normative definitions the rest of the outcome implements against: the per-column-group
output-delta verdict and its transfer rules (`model_properties.md`), and the typed propagation edge
plus the narrowed keyed refusal (`incremental_models.md` §"The graph layer"). Advances success
criteria 1, 2 and 4 by making them stateable; every later phase cites these sections. Spec-only —
no production code changes beyond one new spec-table gate.

## Spec delta

**1. `docs/specs/model_properties.md`**

- §Surface → *Derived proofs*: new row **Output-delta shape**. Verdict
  `OutputDelta` = `AppendOnlyWindow{axis}` ⊑ `KeyedUpsert{keys}` ⊑ `General{reason}`, derived
  **per column group** (the groups §"Per-column mutation-sensitivity / column provenance" already
  factors), never per model — one mutable group must not degrade a model's append-only groups.
  Fail-closed: an operator with no registered transfer rule yields `General{reason}` naming that
  operator. Maturity `not-yet`.
- §Semantics: new section **`### Output-delta shape`** placed after §"Affected-key discovery"
  (it consumes that verdict's key vocabulary). Contents: the lattice and its meaning ("what the
  model emits when its inputs change", not "what its inputs are"); the addressing component
  (window on the output partition axis / key set / whole-table) carried alongside the shape; the
  registered transfer rules, one table row per operator family —
  selection / projection / `UNION ALL` preserve the input shape; keyed aggregation
  (`GROUP BY k`, `DISTINCT k`) over `AppendOnlyWindow` emits `KeyedUpsert{k}`; join emits the meet
  of its inputs' shapes, degraded to `General` on a proven `OneToMany` fan-out; window functions
  and unregistered/unnormalizable operators fail closed to `General`. State that the fold is the
  shared walk's, not a text scan, and that widening is never permitted (a shape may only degrade
  upward through the lattice as the tree is composed).
- §"The composition walk": the existing monotone-lattice bullet names this lattice by its spec
  name rather than the informal `none ⊑ insert-only ⊑ upsert ⊑ general` spelling.
- §Known Divergences: gap-first entry — the verdict is specified but not yet derived; names this
  outcome as the tracking artifact.

**2. `docs/specs/incremental_models.md` §"The graph layer"**

- **Typed edges.** An edge carries a **vector** of typed components, one per column group the
  consumer reads: `(delta shape × addressing × column set)`, the upstream's output-delta verdict
  projected through the consumer's mutation-sensitivity. Today's day-interval dirt is exactly the
  `AppendOnlyWindow` component under window addressing — restate forward propagation and backward
  resolution as the window-addressed case, and state that the adjoint property
  `forward(backward(P)) ⊇ P` continues to hold for that case unchanged. Widen-never-narrow governs
  every addressing: a component whose type cannot be projected degrades to the coarsest component
  the consumer can act on (whole-model dirt), never to nothing.
- **Keyed dirt-sets and the narrowed refusal.** A keyed node without an admitted time axis is no
  longer categorically refused: where its output-delta verdict is `KeyedUpsert{k}`, the edge is
  key-addressed and propagates a **keyed dirt-set** (the affected-key set, §"Affected-key
  discovery"). `MaintenanceGraphUnsupportedNode` survives, narrowed, for a `General` verdict, and
  its message names the operator that degraded the type. Cyclic and self-referential refusals are
  untouched.
- §Known Divergences → *The contract, plan, and graph layer*: gap entry recording that edge typing
  is specified but the propagation layer still carries day intervals only, tracked by this outcome.

No user-visible surface changes land in this phase (`smelt explain` rendering is phase 7), so no
`docs-site/` edit is required here; the docs-site pages that describe propagation are updated in
phase 7 alongside the rendering.

## Tests

- `crates/smelt-logical/tests/output_delta_spec.rs::lattice_levels_are_the_three_named_shapes` —
  §"Output-delta shape" exists and names exactly `AppendOnlyWindow`, `KeyedUpsert`, `General` in
  lattice order; no fourth level sneaks in unannounced.
- `…::transfer_rule_table_is_well_formed` — the transfer-rule table parses: every row has an
  operator family, an input-shape condition, and an output shape drawn from the three levels.
- `…::fail_closed_row_is_present` — a row (or normative sentence) states that an unregistered
  operator yields `General` naming the operator; this is the ratchet phase 2's registry keys off.
- `…::surface_row_exists_for_output_delta` — §Surface *Derived proofs* has an `Output-delta shape`
  row, so the "complete catalogue" constraint is not violated by the new Semantics section.
- `…::graph_layer_states_typed_edges_and_narrowed_refusal` — `incremental_models.md`
  §"The graph layer" mentions the `(shape × addressing × column set)` triple and scopes the
  `MaintenanceGraphUnsupportedNode` keyed-node refusal to `General`.

## Tasks

1. Write `crates/smelt-logical/tests/output_delta_spec.rs` (red: sections absent), modelled on
   `crates/smelt-logical/tests/probe_obligation.rs`'s section/table extraction helpers.
2. Add the §Surface derived-proof row and the §Semantics `### Output-delta shape` section to
   `docs/specs/model_properties.md`; update the composition-walk lattice bullet.
3. Add the Known Divergences gap entry to `model_properties.md`.
4. Rewrite the typed-edge, forward/backward, and refusal paragraphs of
   `docs/specs/incremental_models.md` §"The graph layer"; add its Known Divergences gap entry.
5. Sweep for internal `§"…"` references broken by the new headings and for timeless-oracle
   violations in the new text.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test output_delta_spec` (green)
- `cargo test -p smelt-logical --test probe_obligation --test walk_coverage` (unchanged spec gates
  still parse `model_properties.md` after the edits)
- `rg -n 'Historical name|pre-cut|ratified|category error|Phase [A-Z0-9]' docs/specs/model_properties.md docs/specs/incremental_models.md` — no new hits

## Commit message

`spec(incremental): output-delta shape verdict, transfer rules, and typed propagation edges`
