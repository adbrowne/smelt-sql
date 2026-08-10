# Phase 9 summary — `smelt-db` derives typed model edges

## Shipped

- `crates/smelt-db/src/lib.rs`: `ref_model_edge` no longer hard-codes `output_shape: None`. It
  now derives the upstream's own SQL/sources/skeleton, calls
  `output_delta::derive_output_delta_with_model_verdicts` against a cross-model verdict map, and
  meet-folds the per-column-group result to the edge's scalar `output_shape` — the same pattern
  `smelt-runtime::propagation`'s run-loop derivation uses.
- New private helpers: `resolved_model_sql_and_meta` (frontmatter-stripped SQL + metadata for a
  ref's addressed section, single- or multi-model file), `model_own_source_facts` (a model's own
  `sources.*` refs as `output_delta::SourceFacts`), `model_delta_inputs` (walks model refs
  transitively from a file, address-deduplicated, building one `ModelDeltaInput` per reached
  model — terminates over a cyclic ref graph by construction since the address set is finite).
- New public entry point `smelt_db::model_edges_for(db, workspace, file) -> Vec<ModelEdge>`:
  assembles the cross-model verdict map once (`derive_workspace_output_deltas`) and returns every
  upstream model edge for `file`. `maintenance_plan_report` now calls this instead of inlining
  the assembly, so `smelt explain`'s plan report and any other caller read the same edges.
- `crates/smelt-db/tests/typed_model_edge.rs`: the plan's five tests, all passing — a keyed
  clockless upstream carries `KeyedUpsert`, an existing clocked upstream still carries
  `AppendOnlyWindow` with its clock-based cell unchanged, a phase-8-shaped keyed chain now admits
  a `PerGroupRecompute`/`key_scope` cell instead of a `ReachNotDerivable` refusal, an
  unclassifiable (`SELECT *`) upstream yields `output_shape: None` rather than a fabricated
  shape, and a cyclic model-ref pair terminates fail-closed.
- `docs/specs/incremental_models.md` §Known Divergences: narrowed the "still generic" entry to
  state that the plan report now derives the same typed edges the run loop does — what remains
  unrendered is the affected-key discovery route and the upstream sidecar (phase 10's row).

## Decisions

- Cross-model verdicts are assembled by walking refs transitively from the Salsa `file` (not
  from an eagerly-loaded workspace model list, which `smelt-db` has no equivalent of at this call
  site) — matches the plan's task 3 exactly.
- `derive_workspace_output_deltas` is called ONCE per `model_edges_for` invocation, not once per
  ref or recursively per model reference — the Salsa purity rule's own rationale (a per-ref
  recursive Salsa query could not terminate over a cyclic model-ref graph; the pure fold already
  is bounded-pass cycle-safe).
- Extracted `model_edges_for` as its own `pub fn` (plan didn't mandate this, but the phase's own
  tests need to observe `ModelEdge.output_shape` directly — `MaintenancePlan` does not retain the
  input edges, so there was no other way to pin the derivation without re-deriving it by hand in
  the test).

## For the next planner

- Phase 10 (surface: explain edge rendering, degradation reasons, docs-site update) is now
  unblocked — `model_edges_for`/`ref_model_edge` give it real `output_shape` values to render.
- Not touched, still latent: the `skeleton_closure.rs` `sources.`-only breadcrumb gap (flagged in
  phases 6/7, no fixture has ever tripped it).
- `model_delta_inputs`/`ref_model_edge`'s own output-shape derivation only resolves a
  model-reference leaf that literally spells `smelt.models.<addr>` in SQL text (the walk's own
  `seed_for_leaf_name` only strips a `"models."` prefix) — a bare `smelt.<addr>` model ref (the
  form every current fixture and the run loop's own dag generator use) is not looked up in
  `model_verdicts` at all when appearing *inside* an upstream's own SQL for a 3+-hop chain; it
  falls back through `seed_for_source_name` and degrades to `General` unless that address also
  happens to match a declared source. This is pre-existing behavior from phase 4 (not introduced
  or fixed here) and doesn't affect any current 2-hop fixture or conformance recipe, since
  `upstream_output_delta_groups`-style resolution always derives the immediate upstream's shape
  fresh from its own SQL rather than through the `models.`-prefixed lookup. Flagging in case a
  future 3+-hop keyed-chain conformance recipe trips it.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo
  test`, `example_diagnostics`).
- `cargo test -p smelt-db --test typed_model_edge --test maintenance_model_upstream --test
  maintenance_diagnostics` — 16 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 67 passed.
- `cargo test -p smelt-runtime --test execute_parity --test typed_edge_graph --test
  statement_parity` — 32 passed.
