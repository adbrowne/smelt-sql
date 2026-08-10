# Phase 7 plan — Lowering + execution of a key-addressed model-edge cell

## Objective

Make the key-addressed `PerGroupRecompute` model-edge cell phase 6 derives actually *run*: give it
a statement lowering (affected-key discovery over the upstream's output, key-restricted candidate,
targeted delete+insert) and a driver dispatch path. This closes the "derived but not executed" gap
and is the prerequisite for success criterion 3 (a keyed chain maintained end-to-end, no full-input
rescan) that phase 8's conformance recipes will assert.

## Design calls taken in-plan (not blocked)

- **Affected-key discovery for a model edge is the group-grain fingerprint sidecar over the
  upstream's own output table**, keyed at the upstream's `KeyedUpsert` key columns
  (`diff_repair_group_sidecar_changed_keys`, already shipped for the `MutableSnapshot` source
  posture). A clockless keyed upstream is, from the consumer's view, exactly a mutable snapshot with
  no clock: no interval to clamp with, so `repair_affected_keys_select`'s clamp-less form would
  scan every key and degenerate to a full refresh — which is what criterion 3 forbids. Phase 5's
  keyed dirt channel is symbolic (key columns + provenance), not value-level, so it cannot supply
  the key set; the sidecar can, and works across invocations where the upstream did not run.
  DuckDB-only, matching the existing sidecar posture: a non-DuckDB dialect fails loud by name
  before any backend call, never a silent widening.
- **Key correspondence**: the changed upstream keys are projected through the upstream relation to
  the downstream key columns `KeyScope::keys` names (`SELECT DISTINCT <key_expr(key_scope.keys)>
  FROM <upstream_table> WHERE <upstream key expr> IN (<changed keys>)`). A `key_scope.keys` column
  absent from the upstream relation is a fail-loud refusal, never a widening to every key.

## Spec delta (spec-first — the implement step makes this edit)

`docs/specs/incremental_models.md`:
- §"Upstream model edges" — after the key-addressed paragraph (~L1128), add what such a cell
  *executes*: affected keys discovered by the group-grain sidecar diff over the upstream's output
  at its `KeyedUpsert` key grain, projected to the downstream's key columns; candidate = the
  downstream's full SQL semi-joined to that key relation; write = the repair family's targeted
  `DELETE`+`INSERT`, with the sidecar refreshed transactionally with the write. Name the two
  fail-loud legs (non-DuckDB dialect; a `key_scope` key not carried by the upstream relation).
- §"Known Divergences" — delete the "**A key-addressed model-edge cell is derived and rendered but
  not yet executed**" bullet (~L2358), replaced by a narrowed one if `smelt explain` rendering is
  still absent (that half stays phase 9).

## Tests (red → green)

New `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs`:
1. `key_addressed_cell_resolves_live_from_the_real_plan` — the phase-6 clockless-keyed fixture
   resolves a live key-addressed cell through the driver resolver (not `None`).
2. `execute_wires_output_shape_onto_its_model_edges` — `model_edges_for`'s edges carry a
   `KeyedUpsert` `output_shape` for a keyed upstream (today unconditionally `None`, so the cell is
   never reachable from the driver).
3. `affected_keys_select_restricts_to_the_changed_upstream_keys` — emitter unit test: the SQL
   names only the changed keys and projects `key_scope.keys`, no unrestricted `SELECT DISTINCT`.
4. `missing_key_scope_column_on_the_upstream_fails_loud` — a `key_scope` key absent from the
   upstream relation bails by name rather than widening.
5. `non_duckdb_dialect_refuses_key_addressed_discovery` — refusal before any backend call.
6. `keyed_chain_maintains_only_the_changed_keys_end_to_end` — real DuckDB: two-model chain
   (keyed upstream → keyed consumer), run 1 seeds, run 2 changes one upstream key; the consumer's
   result equals a full refresh **and** untouched key rows are bit-identical (no rewrite).

Existing gates that must stay green unchanged: `keyed_model_edge`, `typed_edge_graph`,
`repair_lowering`, `statement_parity`, `execute_parity`, `maintenance_conformance`.

## Tasks

1. Write the spec delta above first.
2. `smelt-logical::maintenance::emit`: add the key-addressed affected-keys emitter (upstream table
   × upstream keys × `KeyScope::keys` × changed-key literals → `delta_key` relation), mirroring
   `emit_repair_group_digest_select`'s dialect handling. Statement emission stays in `smelt-logical`
   (maintenance-plan purity, single-owner rule); the runtime only composes and executes.
3. `execute.rs::model_edges_for`: derive `output_shape` from the same
   `workspace_output_delta_verdicts` / `upstream_output_delta_groups` fold `propagation.rs` uses —
   no second, independent derivation. (Phase 6 summary "For the next planner".)
4. `maintenance_driver`: `resolve_live_key_addressed_model_edge_cell` — derive the plan via
   `derive_model_maintenance_plan_with_edges`, select the cell whose `key_scope.is_some()`, return
   (edge name, cell, `KeyScope`, upstream key columns, write leg). Fail loud on a `key_scope` key
   the upstream relation does not carry; refuse non-DuckDB before any backend call.
5. `maintenance_driver`: the execution path — `diff_repair_group_sidecar_changed_keys` over the
   upstream's table for changed keys → the new emitter's key relation → `repair_candidate_select`
   over the downstream's full SQL → `execute_per_group_recompute` with `RepairSidecarRefresh` so
   the upstream sidecar advances transactionally with the write. Empty changed-key set = no-op
   (report it, execute nothing).
6. `execute.rs`: dispatch the resolved cell, ordered like the existing repair cell (instead of the
   fold for that trigger, never after it), and never on the creation run.
7. Extend `statement_parity` with a key-addressed model-edge leg (executed-vs-emitted parity +
   the structural no-authoring check).
8. Conditional, red-test-first only if a fixture trips it: `skeleton_closure.rs`'s
   `find_enrichment_join`/`model_edge_enrichment_closure` have the same `sources.`-only breadcrumb
   stripping phase 6 fixed in `relation_matches_source`. Do not fix speculatively.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering --test statement_parity --test execute_parity --test repair_lowering --test typed_edge_graph`
- `cargo test -p smelt-logical --test keyed_model_edge --test walk_coverage`
- `cargo test -p smelt-cli --test maintenance_conformance --test explain_maintenance`

## Commit message

`feat(runtime): lower and execute key-addressed model-edge cells`
