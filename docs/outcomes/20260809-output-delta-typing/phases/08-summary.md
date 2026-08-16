# Phase 8 summary — Conformance recipes: end-to-end keyed chain vs full-refresh oracle

## Shipped

- `crates/smelt-maintenance-testkit/src/dag.rs`: `DagBody::KeyedFold` (a clockless keyed fold,
  `SELECT id, ANY_VALUE(total) FROM <upstream> GROUP BY id`) and `DagBody::PartitionOverKeyedId`
  (a `grain: partition` body that `GROUP BY`s a constant partition column AND the upstream's key
  column, proving row identity through `id` via the walk rather than a declared `unique_key`).
  `NodeGrain::Key` gained a `unique_key: Vec<String>` field (rendered only when non-empty).
  `keyed_chain_dag()` (clockless `KeyedUpsert` → keyed fold) and `keyed_partition_sink_dag()`
  (clockless `KeyedUpsert` → partition-grain downstream, the inert-cell combination) are new
  generated fixtures, added to the module's own smoke tests.
- `crates/smelt-cli/tests/maintenance_conformance/dags.rs`: 4 new tests —
  `keyed_chain_derives_a_typed_keyed_edge` (edge typing), `keyed_chain_fold_matches_full_refresh_oracle`
  and `keyed_chain_maintains_only_the_changed_keys` (real end-to-end fold vs oracle, and the
  no-full-rescan pin, driven through `execute_project` with no run window — these chains have
  none), and `keyed_upstream_partition_downstream_matches_oracle` (correctness pin for the
  flagged inert-cell case).
- `crates/smelt-cli/tests/maintenance_conformance/registry.rs`: `DivergenceStatus::KnownBug` now
  carries `known_bug_keyed_upstream_partition_downstream_no_live_dispatch`, structurally verified
  by a single-call-site grep of `execute.rs`.
- `docs/specs/incremental_models.md` §Known Divergences: a new entry naming the gap and its
  tracking test.

## Decisions

- Declaring `unique_key: id` on a `grain: partition` model alongside its own `timeseries:`
  block trips `GrainAssertionMismatch` (a declared unique key alone reads as key-grain shape
  facts) — `dag_kpart_b` instead PROVES its row identity includes `id` via `GROUP BY d, id` in
  its own body (a structural no-op over an already-one-row-per-id upstream), letting
  `admit_key_addressed_recompute`'s grain proof resolve through `id` without a conflicting
  declaration.
- `keyed_chain_dag`/`keyed_partition_sink_dag` cases drive with `base_request("dev")`
  (`select: vec![]`, no run window) rather than `plan_since_upstream`'s day-interval schedule —
  these chains have no clock at all, mirroring
  `key_addressed_model_edge_lowering.rs::select_request`'s own posture.
- The mutation shape for "a delta touching a subset of keys" is landing MORE rows for a subset
  of already-inserted ids (not an `UPDATE`) — `SUM(val) GROUP BY id` changes for a touched id and
  is bit-identical for an untouched one, without needing raw-SQL mutation outside the testkit's
  `insert_rows` primitive.

## For the next planner

- **`smelt-db`'s own model-edge construction never derives `output_shape`** —
  `crates/smelt-db/src/lib.rs:1385` hard-codes `ModelEdge { output_shape: None, .. }`
  unconditionally. This means `smelt explain`/`maintenance_plan_report`/diagnostics NEVER see a
  `KeyedUpsert` edge, even for the grain:key chain phase 7 proved dispatches correctly at run
  time (verified directly: `classify_node` over `dag_kpart_b` reports `ReachNotDerivable`, not a
  key-addressed cell, because the key-addressed loop in `append_model_edge_cells` silently skips
  when `output_shape` is `None`). Three independent model-edge constructions now exist in the
  codebase — `smelt-runtime::propagation.rs` (correct, used by `build_forward_graph`),
  `smelt-runtime::execute.rs`'s own `model_edges_for` (correct, used at run time), and
  `smelt-db::lib.rs` (stub, used by `smelt explain`/diagnostics). Phase 9 ("Surface: explain edge
  rendering") is exactly the phase that should wire this — flagging here since it's a materially
  bigger gap than phase 9's plan (written before this discovery) may have assumed: it's not just
  that `smelt explain`'s RENDERING of an already-derived edge is generic, but that the edge is
  never derived there at all today.
- The spec delta and registry entry describe the divergence in terms of the run loop's dispatch
  gating (verified directly: exactly one call site of `resolve_live_key_addressed_model_edge_cell`,
  inside the `grain: key` branch) rather than "the plan derives a cell but it's unreached" — the
  latter claim is unverifiable via the production `smelt explain` path today (see the bullet
  above), and this phase's own tests don't route through it either, so the wording was narrowed
  to what's actually checkable.
- No phase-table reshape — phase 9 (surface: explain edge rendering, docs-site update) is
  unaffected in scope, just materially larger than assumed; its own plan should account for the
  `smelt-db` gap above.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test maintenance_conformance` — 67 passed.
- `SMELT_CONFORMANCE_CASES=24 cargo test -p smelt-cli --test maintenance_conformance dags` — 9
  passed (soak).
- `cargo test -p smelt-runtime --test since_upstream_propagation --test typed_edge_graph --test key_addressed_model_edge_lowering --test statement_parity` — 52 passed.
- `cargo test -p smelt-logical --test walk_coverage --test keyed_model_edge` — 9 passed.
