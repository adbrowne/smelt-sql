# Phase 8 plan — Conformance recipes: end-to-end keyed chain vs full-refresh oracle

## Objective

Lift phase 7's hand-typed two-model proof into the **standing generative conformance gate**
(`crates/smelt-cli/tests/maintenance_conformance/dags.rs`, over
`smelt-maintenance-testkit`'s `DagRecipe` pool): a generated keyed chain — clockless
`KeyedUpsert` upstream → keyed consumer that folds it — is driven through the real
`execute_project` pipeline and compared node-by-node against an independently staged
full-refresh twin, plus a no-full-rescan assertion. Advances success criteria 3 (end-to-end
incremental chain in the conformance gate), 4 (keyed dirt-sets over the narrowed refusal,
observed on a *generated* graph) and 6 (standing gates green, pool widened).

## Spec delta

`docs/specs/incremental_models.md` §"Known Divergences" — add one entry: a `KeyedUpsert`
upstream feeding a **partition-grain** downstream derives a key-addressed model-edge cell in
the maintenance plan, but the run loop has no live dispatch for that combination, so the
downstream maintains via its ordinary window route (correct, not incrementally folded). Names
`execute.rs`'s window-forward branch as the site and this outcome as the tracking artifact.

## Tests (red → green)

1. `dags.rs::keyed_chain_derives_a_typed_keyed_edge` — over the generated `keyed_chain_dag()`,
   `build_forward_graph` derives an edge into the keyed downstream carrying a `KeyedUpsert`
   component with `Addressing::Keyed`; the existing
   `keyed_grain_node_excluded_from_generated_graph` (append-only upstream) must stay green —
   the refusal is narrowed, not removed.
2. `dags.rs::keyed_chain_fold_matches_full_refresh_oracle` — stage the chain twice
   (`stage_pair`), initial run both, land a generated delta mutating a subset of upstream keys,
   run the inc project through `execute_project`, rebuild the full twin over the combined
   history; `assert_every_node_equal` for every node, every step.
3. `dags.rs::keyed_chain_maintains_only_the_changed_keys` — after the incremental step, a
   downstream row whose upstream key was untouched is bit-identical to its pre-step value
   (`fetch_node_multiset` before/after), while a touched key's row moved: the gate-level
   analogue of phase 7's "no full-input rescan".
4. `dags.rs::keyed_upstream_partition_downstream_matches_oracle` — the
   `keyed_partition_sink_dag()` combination phase 7 flagged as having no live dispatch is
   still multiset-equal to the oracle (correctness pin for the inert-cell case).
5. `registry.rs::divergence_registry_staleness_report` — extend with a `KnownBug` entry
   `known_bug_keyed_upstream_partition_downstream_no_live_dispatch`, whose
   `known_bug_still_reproduces` structural check greps `smelt-runtime/src/execute.rs` for the
   fact that `resolve_live_key_addressed_model_edge_cell` is consulted only from the keyed run
   loop (single call site); the entry goes stale the moment the dispatch widens.

## Tasks

1. Spec edit first: the §"Known Divergences" entry above.
2. `smelt-maintenance-testkit/src/dag.rs`: add `DagBody::KeyedFold`
   (`SELECT id, ANY_VALUE(total) AS total FROM <upstream> GROUP BY id`), wiring it through
   `node_output_columns` and `render_node_body`.
3. Same file: add `keyed_chain_dag()` — clocked `events` source → `dag_kchain_a`
   (`DagBody::KeyedAgg`, `NodeGrain::Key`) → `dag_kchain_b` (`DagBody::KeyedFold`,
   `NodeGrain::Key`), mirroring phase 7's proven `agg` → `downstream` shape.
4. Same file: add `keyed_partition_sink_dag()` — `dag_kpart_a` (`KeyedAgg`, `NodeGrain::Key`)
   → `dag_kpart_b` (`AdditiveAgg`, `NodeGrain::Partition`). Verify `stage_dag` renders a keyed
   node with no `timeseries:` block (existing `NodeGrain::Key` behaviour) and that a
   downstream reading a keyed upstream still renders a valid body.
5. `dags.rs`: a keyed-chain driving helper — these chains have **no run window**, so drive
   with a model-selecting `ExecuteRequest` (mirroring
   `smelt-runtime/tests/key_addressed_model_edge_lowering.rs::select_request`), never
   `plan_since_upstream`'s day-interval schedule. Mutate upstream rows via `insert_rows` /
   direct source mutation for a subset of keys only.
6. Write tests 1–4 red-first against that helper; make them green. Any oracle divergence is a
   production bug to fix in-phase, never a registry entry.
7. Add the registry entry + structural check (test 5).
8. Confirm the new DAG cases run inside the default `SMELT_CONFORMANCE_CASES` budget without
   materially slowing the gate; if a case is expensive, keep its generated window count at the
   `DEFAULT_CASES = 6` scale rather than raising the default.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test maintenance_conformance` (the standing gate, all modules)
- `SMELT_CONFORMANCE_CASES=24 cargo test -p smelt-cli --test maintenance_conformance dags`
  (deeper soak of the new recipes; local only, not the standing budget)
- `cargo test -p smelt-runtime --test since_upstream_propagation --test typed_edge_graph
  --test key_addressed_model_edge_lowering --test statement_parity`
- `cargo test -p smelt-logical --test walk_coverage --test keyed_model_edge`

## Commit message

`test(conformance): generated keyed chain folded end-to-end against the full-refresh oracle`
