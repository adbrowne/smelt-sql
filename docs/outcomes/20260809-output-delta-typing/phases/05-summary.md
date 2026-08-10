# Phase 5 summary — Keyed dirt-set propagation for admitted shapes

## Shipped

- `crates/smelt-logical/src/maintenance/propagate.rs`: `classify_keyed_edges`/`classify_keyed_edge`
  replace `refuse_keyed_nodes` — per edge touching a `PartitionGrain::Keyed` endpoint, admit iff a
  component carries `Addressing::Keyed`; else refuse (naming the degrading operator for a
  `WholeModel` component, or the verbatim old bare-keyed wording when the vector is empty —
  fail-closed).
- New `KeyedDirt { keys, from }` and `Propagation::{per_edge_keys, keyed_dirty}` — an additive
  keyed channel alongside the existing interval maps. `propagate` routes an admitted keyed edge
  into it; a downstream that is itself keyed-grain stays on the keyed channel alone, a clocked/
  unclocked consumer also gets `DayInterval::WHOLE` (widen-never-narrow, never nothing).
  `required_inputs` resolves an admitted keyed edge's upstream requirement to `WHOLE` (no clamp
  arithmetic on a nonexistent axis).
- `crates/smelt-runtime/src/propagation.rs`: `refuse_bare_keyed_origins` now consults
  `workspace_output_delta_verdicts` — a `--source`/`--landed` origin naming a bare keyed model
  admits when its own derived shape has at least one non-`General` column group; refuses only on
  `General`/absent.
- Spec: `docs/specs/model_properties.md` (Surface row + Known Divergences bullet) and
  `docs/specs/incremental_models.md` (Known Divergences bullet) updated to state the acted-on
  behaviour and the narrowed remaining gap (keyed dirt-sets are symbolic — key columns +
  provenance — not a materialised key-value set).
- 9 tests per the plan: 7 in `maintenance_propagation_adjoint.rs` (pure composition math), 1 in
  `typed_edge_graph.rs::keyed_upstream_model_propagates_on_the_real_graph` (real graph builder), 1
  in `since_upstream_propagation.rs::bare_keyed_origin_refusal_narrows_to_general`.

## Decisions

- Kept the pre-phase-5 **eager whole-graph scan**: `classify_keyed_edges` classifies every edge up
  front, so a workspace containing even one unaddressed keyed edge still fails closed on ANY
  `propagate`/`required_inputs` call, regardless of whether that edge is reached by the current
  call's dirt — this is why `bare_keyed_origin_refusal_narrows_to_general`'s two scenarios use
  separate temp workspaces (a `General`-shaped model anywhere in the same workspace would poison
  an unrelated admitted-origin call otherwise).
- A bare keyed node never gets an *inbound* edge from its own driving source unless that source is
  itself unclocked/mutable (confirmed empirically against `keyed_grain_model_never_derives_an_edge`
  and `bare_keyed_upstream_still_refuses`) — a plain `GROUP BY` over one append-only source with no
  other join yields zero plan cells for the keyed model's own creation, so its only edges are
  *outbound* to its own downstream consumers. `keyed_upstream_model_propagates_on_the_real_graph`
  gives the keyed model a synthetic `timeseries:` block (declared but not locality-admitted) purely
  so the downstream's `ModelEdge` derivation gets a `clock_col` and produces a walkable edge —
  documented in the test's own fixture rather than the spec, since it's a plan-derivation mechanic
  not new behaviour.
- Updated (rather than left verbatim) the pre-existing `bare_keyed_upstream_still_refuses`
  integration test: its scenario always had a real `dims`-sourced `WholeModel` component on the
  inbound edge, so phase 5 correctly produces the more specific General-degrade message instead of
  the old bare-keyed one. This is a distinct edge from the "no components at all" case the plan's
  test 3 pins verbatim (`maintenance_propagation_adjoint.rs` ll. 303–320-equivalent), which is
  unchanged.

## For the next planner

- Phase 6 (conformance recipes: end-to-end incremental chains vs full-refresh oracle) is now
  unblocked — the blanket keyed refusal that stopped a keyed upstream from reaching its consumer
  is gone for admitted shapes.
- Not addressed here (unchanged from phase 4, still open, not this phase's success criterion):
  `referenced_column_names`'s name-only column-group disambiguation, and
  `derive_workspace_output_deltas`'s `O(models^2)` worst case — both already tracked under Out of
  scope in `outcome.md`.
- `smelt explain`'s own edge rendering (success criterion 5) is still phase 7's job, alongside the
  docs-site update — the keyed channel now exists on `Propagation` for it to read from.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo test`
  workspace, `example_diagnostics`).
- `cargo test -p smelt-logical --test maintenance_propagation_adjoint --test typed_edge_spec --test output_delta_spec --test output_delta_workspace --test walk_coverage` — 36 passed.
- `cargo test -p smelt-runtime --test typed_edge_graph --test since_upstream_propagation --test tracer_propagation --test statement_parity --test execute_parity` — 53 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 63 passed.
