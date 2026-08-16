# Phase 2 plan — dispatch the key-addressed repair cell outside `grain: key`

## Objective

Make the derived key-addressed model-edge cell actually run for a `grain: partition`
downstream of a clockless `keyed upsert` upstream. Today `derive.rs::append_model_edge_cells`
already admits the cell for that shape (the clock route does not apply), but the only dispatch
site lives inside `execute.rs`'s `if plan_is_keyed` branch, so the cell is inert and the model
maintains via the correct-but-whole ordinary route. Advances success criterion 1 (and feeds 7).

## Spec delta

No new surface — the behaviour is already pinned by `incremental_models.md` §"Dispatch — from
propagated components to run units" ("dispatch is keyed by the component's addressing, never by
the downstream model's grain") and §"The graph layer" → "Upstream model edges". The truth edit
is in §Known Divergences: the "scheduler does not yet consume delta signatures end to end"
bullet's **first** clause (the `grain: partition` inert-cell sentence) is removed, leaving the
key-valued-dirt and watermark clauses intact, and gaining one sentence naming the residue this
phase leaves: a partition-grain downstream that *also* has an inbound edge/source the
key-addressed cell does not cover keeps the ordinary route (tracked to phase 3).

## Tests

Red-green, in this order.

1. `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs::
   partition_grain_downstream_dispatches_key_addressed_cell` — reuse `stage_chain_project`'s
   harness with `downstream` restaged as `grain: partition` (`partition_by: d` over a constant
   `d`, `GROUP BY d, user_id` — the `DagBody::PartitionOverKeyedId` shape, so the walk proves
   `user_id` row identity without a declared `unique_key`). After mutating user 1's upstream
   contribution, the second run's `RunOutcome` record for `downstream` has
   `strategy == "per_group_recompute"` and user 1's row is repaired while user 2's is untouched.
   **RED today** (the ordinary route runs; strategy is not the repair label).
2. `…::partition_grain_downstream_with_an_uncovered_input_keeps_the_ordinary_route` — same
   downstream plus a second inbound ref (a clocked declared source) the key-addressed cell does
   not cover: the run keeps its ordinary route and stays multiset-correct. Pins the
   substitution gate so the fix can never silently drop a component it does not dispatch.
3. `…::key_addressed_cell_resolves_for_a_partition_grain_downstream` — unit leg over
   `resolve_live_key_addressed_model_edge_cell` with `grain: partition` metadata: the cell,
   its `KeyScope` and its digest columns resolve identically to the `grain: key` leg
   (characterization pin that the gap is dispatch-only, not derivation).
4. `crates/smelt-cli/tests/maintenance_conformance/dags.rs::
   keyed_upstream_partition_downstream_matches_oracle` — extend the existing oracle test with
   an incrementality assertion on the repair run: `dag_kpart_b`'s manifest strategy is
   `per_group_recompute` (correctness assertion unchanged).

## Tasks

1. Write test 1 and watch it fail on the strategy label (record the actual label in the summary).
2. In `execute.rs`, hoist the inputs the key-addressed resolution needs (`clean_sql_for_merge`,
   `build_maint_source_facts`, `table_exists_before_run`, `db_table_name`, `model_edges_for`)
   above the `if plan_is_keyed` gate at ~L1863 and resolve `key_addressed_edge_cell` once, so
   both branches read the SAME resolved cell (no second derivation — maintenance-plan purity).
3. Extract the keyed branch's key-addressed execution arm (~L2030–L2100) into one helper
   (`dispatch_key_addressed_model_edge`) returning the `ExecutionResult` + the resolved
   `RepairWrite`, and call it from the existing keyed site unchanged.
4. Add the non-keyed dispatch site immediately before the
   `match plan.incremental.as_ref()…` at ~L2703: when the cell resolved, the table already
   exists, and the **substitution gate** holds, call the helper, write the manifest entry with
   strategy `per_group_recompute`/`diff_patch`, run check seam A's equivalent, and return
   `ModelOutcome::Completed`.
5. Substitution gate (phase-2 scope, fail-safe): substitute for the ordinary route only when
   every inbound ref of the model is a key-addressed model edge that resolved a cell — i.e. the
   model has no declared sources and no non-key-addressed model edge. Otherwise keep the
   ordinary route; document in the code comment that this is the widen-never-narrow leg and
   that the composed multi-component case is phase 3's.
6. Update the stale doc comments that assert the cell is inert:
   `crates/smelt-maintenance-testkit/src/dag.rs::keyed_partition_sink_dag` and the
   `keyed_upstream_partition_downstream_matches_oracle` doc comment.
7. Land tests 2–4, then the §Known Divergences narrowing described above.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches

## Commit message

`feat(incremental): dispatch key-addressed repair cells outside the grain: key run branch`
