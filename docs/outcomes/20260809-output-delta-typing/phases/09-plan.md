# Phase 9 plan — `smelt-db` derives typed model edges (`ModelEdge.output_shape`)

## Objective

Make the Salsa layer that backs `smelt explain` and the maintenance diagnostics derive the same
typed model edges the run loop already derives: `ref_model_edge` in `crates/smelt-db/src/lib.rs`
currently hard-codes `output_shape: None`, so `append_model_edge_cells`' key-addressed loop
silently skips and no `KeyedUpsert` edge is ever visible to reporting. Advances criterion 5 (the
prerequisite for rendering an edge's delta type — phase 10 renders it) and criterion 6 (the plan
report stops diverging from the run loop for keyed chains).

## Spec delta

`docs/specs/incremental_models.md` §Known Divergences, the entry **"A key-addressed model-edge
cell's `smelt explain` rendering is still generic."** — narrow it to rendering only: state that
`smelt explain`'s plan report derives the same typed model edges (upstream output-delta shape per
edge) the run loop does, and that what remains unrendered is the affected-key discovery route and
the upstream sidecar it reads. Keep the tracking link. No other section changes: §"Upstream model
edges" already specifies the key-addressed admission rule this phase makes reachable from the
Salsa path; this phase implements the spec, it does not change it.

## Tests

New file `crates/smelt-db/tests/typed_model_edge.rs` (fixtures in the style of
`maintenance_model_upstream.rs`, driving `smelt_db::maintenance_plan_report`):

- `keyed_upstream_edge_carries_keyed_upsert_output_shape` — a clockless `grain: key` upstream
  (`SELECT id, SUM(v) ... GROUP BY id`) referenced by a downstream: the derived `ModelEdge` for
  that upstream has `output_shape: Some(OutputDelta::KeyedUpsert { .. })`.
- `clocked_append_only_upstream_edge_carries_append_only_window` — an existing-shaped clocked
  upstream chain still types as `AppendOnlyWindow` and its clock-based cell is unchanged (no
  regression of the pre-existing route).
- `keyed_chain_plan_report_admits_a_key_addressed_cell` — over a fixture shaped like phase 8's
  `keyed_chain_dag` (clockless keyed upstream → `grain: key` downstream), the plan report now
  reports a key-addressed `Technique::PerGroupRecompute` cell instead of a
  `ReachNotDerivable` refusal. This is the phase-8 summary's directly-verified gap, red first.
- `unresolvable_upstream_yields_no_output_shape` — an upstream this workspace cannot locate
  contributes `output_shape: None`, never a fabricated or optimistic shape (fail-closed).
- `cyclic_model_refs_terminate_fail_closed` — two models referencing each other terminate (no
  hang, no panic) with a `General`-or-`None` shape, pinning the bounded-pass property of the
  chosen non-recursive assembly.

## Tasks

1. Read `analysis::output_delta::{ModelDeltaInput, derive_workspace_output_deltas, SourceFacts}`
   and `smelt-runtime::propagation::{workspace_output_delta_verdicts, upstream_output_delta_groups,
   model_output_delta_sources}` — the Salsa assembly must produce the same inputs, not a variant.
2. Write the five tests above (red).
3. Add a private helper in `crates/smelt-db/src/lib.rs` (near `ref_model_edge`) that, from the
   downstream `file`, walks model refs transitively with a visited set via `resolve_ref_path`
   (`RefKind::Model` only), and builds one `ModelDeltaInput` per reached model: `address` =
   the ref's `smelt.`-stripped path lowercased to match `derive_workspace_output_deltas`' keying,
   `sql` = the model's frontmatter-stripped text, `ctx` = `JoinContext::new()`, `sources` =
   that model's own `smelt.sources.*` refs resolved through the existing `ref_source_info` and
   mapped with `SourceFacts::from_source_info` (bare name, `sources.` stripped — mirroring
   `model_output_delta_sources`).
4. Call `derive_workspace_output_deltas` ONCE on that input set (cycle-safe by construction — do
   not recurse a Salsa query per model reference).
5. In `ref_model_edge`, resolve the upstream's per-`ColumnGroup` verdicts via
   `derive_output_delta_with_model_verdicts` (sql/sources/skeleton from the upstream's own
   metadata, using `skeleton_columns` exactly as `model_skeleton_columns` does) against that map,
   `meet`-fold to a scalar, and set `output_shape`. Replace the stub comment; `None` stays only
   for an upstream contributing no groups.
6. Thread the assembled map so `maintenance_plan_report` does the fold once per report, not once
   per ref, if the per-ref call shows up as gratuitous re-derivation.
7. Apply the spec delta.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy zero-warnings, full `cargo test`,
  `example_diagnostics`)
- `cargo test -p smelt-db --test typed_model_edge --test maintenance_model_upstream --test maintenance_diagnostics`
- `cargo test -p smelt-cli --test maintenance_conformance` (the plan-report path feeds it)
- `cargo test -p smelt-runtime --test execute_parity --test typed_edge_graph --test statement_parity`

## Commit message

`feat(smelt-db): derive typed model edges so explain sees keyed-upsert upstreams`
