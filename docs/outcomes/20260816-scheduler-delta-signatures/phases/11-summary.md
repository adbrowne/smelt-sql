# Phase 11 summary — walk fix: `GROUP BY` keys resolve against output aliases

## Shipped

- `resolve_group_by_key_to_output(items, key)` — new shared leaf classifier in
  `crates/smelt-logical/src/analysis/mod.rs`: resolves a `GROUP BY` key to its output-column
  name via expression-text match (exact), then output-alias match (case-insensitive). Single
  owner for both call sites.
- `analysis::walk::PropertyTransfer::group_by_output_keys` (`walk.rs`) now calls the shared
  helper instead of only matching expression text — grouping by a projected alias (e.g.
  `SELECT date_trunc('day', ts) AS d, user_id, … GROUP BY d, user_id`) now proves grain/row
  identity for the whole scope instead of failing closed to unkeyed.
- `analysis::mod::scope_group_by_alignment` (partition-alignment check, feeds `HAVING`/window
  admission) fixed the same way — alias grouping now reports `Aligned` when it should.
- Spec deltas: `docs/specs/model_properties.md` §"Region row identity" and
  `docs/specs/incremental_shapes.md` §"Safety checks" both gained the resolution-route sentence
  (expression text / output alias / ordinal).
- Tests 1–6 from the plan, all green: `group_by_alias_resolves_to_output_key`,
  `group_by_expression_text_still_resolves` (regression, covers both text and ordinal legs),
  `group_by_non_projected_key_still_fails_closed`, `group_by_alias_match_is_case_insensitive`
  (all in `walk.rs`'s test module), `scope_group_by_alignment_accepts_alias_grouping` (in
  `mod.rs`'s test module), `alias_grouped_model_proves_row_identity` (new integration test in
  `crates/smelt-db/tests/maintenance_ledger.rs`, full Salsa path).

## Decisions

- Task 6 (revisit the two phase-2 workaround sites) hit the **"different reason" branch the
  plan called out in advance**: `crates/smelt-maintenance-testkit/src/dag.rs`
  (`DagBody::PartitionOverKeyedId`) and `crates/smelt-runtime/tests/
  key_addressed_model_edge_lowering.rs` (`stage_chain_project_partition_downstream`) both fail
  when restored to the honest `GROUP BY {d}, {id}` shape — not from the walk anymore (it now
  proves grain `[d, id]` correctly), but because `derive_affected_keys` then names `d` in the
  key-addressed cell's `key_scope`, and `d` is absent from the upstream's own proven
  `KeyedUpsert` key columns (`[id]` only) — `MaintenanceKeyScopeColumnMissing` refuses the cell.
  Reverted both to the original constant-projection shape (`GROUP BY {id}` alone, `d` a literal
  outside `GROUP BY`) and rewrote every comment that named the now-false walk explanation to
  name the real remaining cause, per the plan's explicit instruction not to widen scope into
  `derive_affected_keys` here.
- Confirmed via `cargo test -p smelt-cli --test maintenance_conformance
  keyed_upstream_partition_downstream_matches_oracle` and `cargo test -p smelt-runtime --test
  key_addressed_model_edge_lowering` that the reverted shapes are back to green before moving on.

## For the next planner

- `derive_affected_keys` returning every grain column into `KeyScope` (rather than intersecting
  with the upstream's own proven key columns) is a real, reproducible gap — it blocks the
  honest alias-grouped `GROUP BY d, id` shape for a `grain: partition` downstream of a clockless
  keyed upstream. Two call sites now carry a precise comment naming the exact diagnostic
  (`MaintenanceKeyScopeColumnMissing`) and message shape. Worth a dedicated phase if phase 12's
  conformance recipes want this combination; not required to serve this phase's own criteria.
- Phase 12 (conformance extension) can now write recipes using ordinary alias grouping
  (`GROUP BY d, user_id` where `d` is a real derived column, not a workaround) instead of the
  constant-projection trick — as long as they don't also require the upstream's key_scope to
  include a column absent from the upstream's own key, per the note above.

## Gates

- `cargo test -p smelt-logical --quiet` — pass (includes `walk_coverage`).
- `cargo test -p smelt-db --test maintenance_ledger --test maintenance_signature --quiet` — pass.
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering --quiet` — pass.
- `cargo test -p smelt-cli --test maintenance_conformance keyed_upstream_partition_downstream_matches_oracle --quiet` — pass (spot-checked the reverted dag.rs shape).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full `cargo test`, `example_diagnostics`).
