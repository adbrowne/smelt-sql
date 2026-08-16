# Phase 10 plan — Surface: explain edge delta types, degradation reasons, docs-site

## Objective

Make `smelt explain <model>`'s plan report show, per inbound edge, the derived output-delta type
and — when it is `general` — the construct or world-fact that degraded it. Phase 9 made
`ModelEdge.output_shape` real in the Salsa layer; this phase renders it (plus the source-edge leaf
seed, so *every* inbound edge is typed, not only model edges) and syncs the spec Surface and
docs-site. Advances success criteria 5 and 6.

## Spec delta

`docs/specs/incremental_models.md`:

- §Surface → "CLI", the `smelt explain <model>` bullet — state that the inbound-edges section
  prints each edge's derived **output-delta type** (`append-only within window` / `keyed upsert` /
  `general`), and that a `general` edge names the construct or world-fact that degraded it (an
  unregistered operator, a row-multiplying join, a source with no declared `mutation_profile`).
  Source edges are typed by their declared mutation profile; model edges by the upstream's own
  derived verdict (§"Upstream model edges").
- §Surface → same bullet — a key-addressed model-edge cell prints its affected-key discovery
  mechanism as the group-grain fingerprint sidecar over the upstream's output table (the third
  discovery posture alongside the clamped current-source scan and the source sidecar diff).
- §Known Divergences — delete the "**A key-addressed model-edge cell's `smelt explain` rendering
  is still generic**" entry (this phase closes both halves it names). Leave the
  `grain: partition` dispatch divergence untouched.

## Tests

Red-green, all in `crates/smelt-cli/tests/explain_maintenance.rs` unless noted.

1. `explain_renders_keyed_upsert_edge_delta_type` — clockless keyed upstream ⇒ that edge's block
   prints `delta type: keyed upsert`.
2. `explain_renders_append_only_window_edge_delta_type` — an existing clocked append-only upstream
   still prints `delta type: append-only within window` (no regression in the common case).
3. `explain_names_construct_that_degraded_edge_delta_type` — an upstream the walk cannot classify
   ⇒ `delta type: general` plus a reason substring naming the offending construct.
4. `explain_renders_source_edge_delta_type` — a source edge with no declared `mutation_profile`
   prints `general` and says so in the reason (fail-closed seed is visible, not silent).
5. `explain_key_addressed_cell_prints_upstream_sidecar_discovery` — a key-addressed model-edge
   cell's repair stanza names the group-grain fingerprint sidecar over the upstream.
6. `explain_edge_without_derived_shape_prints_no_delta_row` — `output_shape: None` renders no
   fabricated type (absence stays absence).

## Tasks

1. Spec edit above (spec-first), including the §Known Divergences deletion.
2. Add an `edge_delta_types: &[(String, OutputDelta)]` parameter to
   `build_maintenance_plan_report` (`crates/smelt-cli/src/explain.rs`) — already-derived data,
   never re-derived in the string builder — and render a `delta type:` row inside each edge's
   block after its contract rows; `General { reason }` renders as
   `general (degraded by: <reason>)`; a missing entry renders nothing.
3. In `crates/smelt-cli/src/commands/explain.rs`, assemble that vector: model edges from
   `smelt_db::model_edges_for(&db, ws, file)` (`ModelEdge.output_shape`), source edges from
   `output_delta::seed_shape_for_source` over the single-owner
   `smelt_db::queries::maintenance::source_facts`. Match `ModelEdge.name` (address with `smelt.`
   stripped, possibly carrying a `models.` breadcrumb) to `InboundEdgeContract.name` (canonical
   path) by stripping the breadcrumb on the edge side — assert the mapping in test 1 rather than
   assuming it.
4. Update `build_report_for` in `explain_maintenance.rs` (and any other
   `build_maintenance_plan_report` caller — `explain_model.rs`, `explain_show_sql.rs`) for the new
   parameter, passing the real assembled vector so the tests exercise the production path.
5. Render the key-addressed cell's discovery-mechanism line in the existing repair stanza
   (the cell carries `key_scope`; the mechanism is the upstream fingerprint sidecar from phase 7)
   — reuse the stanza's existing wording shape, do not invent a second stanza.
6. `docs-site/docs/reference/cli.md` — extend the `smelt explain` sample output's `Inbound edges`
   block with the new `delta type:` rows and add one sentence explaining the three types and what
   a `general` reason tells the modeller.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt + clippy + full `cargo test` + example_diagnostics).
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test explain_show_sql`
- `cargo test -p smelt-db --test typed_model_edge` (edge derivation unchanged by the new caller).
- `cargo test -p smelt-logical --test walk_coverage --test output_delta_spec` (spec-table sync).

## Commit message

`feat(explain): render each inbound edge's output-delta type and degradation reason`
