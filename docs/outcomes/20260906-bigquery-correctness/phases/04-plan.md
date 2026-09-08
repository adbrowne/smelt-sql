# Phase 4 plan — the enrichment-keyed model-edge route, and a loud refusal behind it

## Objective

Derive the maintenance cell that today does not exist: a clockless keyed upstream model
read in **value-enrichment** position by a partition-addressed downstream must contribute
an `UpstreamMutation` cell (`Corner::ColumnMerge`, `Technique::ColumnScopedMerge`) over
exactly the downstream columns that edge provides, addressed by the join key the downstream
carries — the same technique smelt already derives for a declared `mutation_profile:
mutable_snapshot` dimension. Where that route does not admit, the surviving
`RepairKeysNotDiscoverable` refusal gains a real `DiagnosticCode` so it is loud at
`build`/`run` instead of visible only through `smelt explain --json`. Advances success
criteria 2 and 3; sets up criterion 5's `gold_events_enriched` divergence (phase 5 lands
the run-path half).

## Spec delta (make this edit first)

- `docs/specs/incremental_models.md` §"Upstream model edges" — after the paragraph
  describing the two key-addressed discovery routes, add the **enrichment-keyed** route:
  a clockless `keyed upsert` upstream joined in enrichment position (a join whose `ON`
  predicate matches the upstream's own declared `unique_key`, contributing payload columns
  the downstream SELECTs) by a **partition-addressed** downstream contributes a
  `Trigger::UpstreamMutation` cell with `Technique::ColumnScopedMerge` over the
  edge-provenanced column group, addressed by the join key columns the downstream itself
  projects — not by the downstream's grain. Scan bounds: the cell is
  `PartitionLocal::No` (a rename scatters across every output partition), so it requires
  the edge's declared `maintenance.scan_bounds.per_source.<edge>.allow_full_scan`, else
  `MaintenanceScanUnbounded`. State the ordering: this route is attempted only after the
  key-addressed route declines, and `MaintenanceRepairKeysNotDiscoverable` now fires only
  when **both** decline.
- `docs/specs/diagnostics.md` — `MaintenanceRepairKeysNotDiscoverable`'s catalogue row
  already exists; move it out of the §Known Divergences "no `DiagnosticCode` variant yet"
  sentence (leave `MaintenanceRepairSliceUnbounded` there) and record it as implemented.

## Tests (red first)

`crates/smelt-logical/tests/model_edge_enrichment_mutation.rs` (new):
1. `clockless_keyed_dimension_yields_column_scoped_mutation_cell` — a partition-addressed
   downstream LEFT JOINing a clockless `KeyedUpsert` edge on the edge's `unique_key`, with
   `allow_full_scan` on that edge, derives exactly one `Trigger::UpstreamMutation` cell,
   `Technique::ColumnScopedMerge`, and no `RepairKeysNotDiscoverable` refusal.
2. `mutation_cell_group_is_only_the_edge_provenanced_columns` — the cell's `group` names
   `current_repo_name` alone, never a passthrough column of the fact relation.
3. `mutation_cell_key_scope_is_the_join_key_carried_in_the_output` — the cell's
   `key_scope` names `repo_id` (projected by the downstream), not the downstream's grain.
4. `enrichment_route_refuses_when_the_join_key_is_not_projected` — same shape with the
   join key dropped from the SELECT list ⇒ no cell, `RepairKeysNotDiscoverable` naming the
   edge (fail-closed, not a whole-table cell).
5. `enrichment_route_refuses_an_unbounded_scan_without_allow_full_scan` — same shape
   without `allow_full_scan` ⇒ `Refusal::ScanUnbounded` naming the edge, no cell.
6. `clocked_edge_keeps_todays_delete_insert_route` — a clocked upstream is unchanged
   (regression guard: the new route never widens an already-admitted edge).
7. `membership_position_edge_gets_no_column_scoped_cell` — the edge read in row-admission
   position (INNER JOIN governing which rows exist) is not eligible for a column-scoped
   merge; it stays refused rather than silently value-merged.

`crates/smelt-db/tests/maintenance_diagnostics/` (extend the existing suite):
8. `repair_keys_not_discoverable_raises_a_diagnostic` — a model whose model edge declines
   every route emits `MaintenanceRepairKeysNotDiscoverable` from `file_diagnostics()`.
9. `refusal_codes` integration test extended so `refusal_code(RepairKeysNotDiscoverable)`
   returns the new name and matches the code the pipeline emits (the existing
   `refusal_code_names_are_real_variants` gate then covers it).

`crates/smelt-cli/tests/github_activity_replay.rs`:
10. `events_enriched_dimension_mutation_cell_technique` — flip this existing measured-
    verdict test from "no `UpstreamMutation(gold.repo_dim)` cell is derived" to asserting
    the cell exists with `ColumnScopedMerge`, via `smelt explain --json`.

## Tasks

1. Make the two spec edits above.
2. Add `Refusal::RepairKeysNotDiscoverable`'s `DiagnosticCode` variant
   (`crates/smelt-db/src/diagnostics_types/mod.rs`), map it in
   `crates/smelt-db/src/queries/maintenance/refusal_diag.rs` and
   `diagnostics.rs` (replacing the `None` arm), and return its name from
   `refusal_code` (`crates/smelt-logical/src/maintenance/refusal.rs`).
3. In `crates/smelt-logical/src/maintenance/derive/model_edge.rs`, add a pure helper that,
   for one clockless `KeyedUpsert` edge, derives (a) the edge-provenanced column group by
   calling the single-owner `grouping::derive_column_groups` over synthesized
   `SourceFacts { name: edge.name, mutation: MutableSnapshot, partition_col: None,
   unique_key: edge.unique_key, allow_full_scan }` plus
   `skeleton::skeleton_columns(sql, &[], output_partition_col)`, and (b) the join key
   columns, from the existing enrichment-join machinery
   (`analysis::skeleton_closure::enrichment_join_alias` / the `JoinContext` already built
   in this function), keeping only keys the downstream's own SELECT list projects.
4. Wire that helper as a third route in the key-addressed loop: on
   `Err(RepairRefusal::KeysNotDiscoverable { .. })`, attempt it before pushing the refusal;
   push either the `ColumnScopedMerge` cell, a `Refusal::ScanUnbounded` (no
   `allow_full_scan`), or the original refusal. Membership-sensitive groups are excluded
   (test 7). No change to the clocked route.
5. Thread the edge's `allow_full_scan` into `ModelEdge` (new field, defaulted at every
   existing construction site) so step 4 can read the declared scan-bound acceptance;
   populate it in `crates/smelt-db/src/queries/maintenance/plan.rs`'s
   `derive_model_maintenance_plan_with_edges` from `metadata.maintenance.scan_bounds`.
6. Update `examples/github_activity/models/gold/events_enriched.sql`'s header comment: the
   measured "NO `UpstreamMutation` cell is derived" finding is now fixed at the derivation
   layer, with the run-path half named as phase 5.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test model_edge_enrichment_mutation --test maintenance_referential_integrity --test model_edge_delta_restriction`
- `cargo test -p smelt-db --test maintenance_diagnostics --test integration`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance --test github_activity_replay --features duckdb`
- `cargo test -p smelt-cli --test example_diagnostics`

## Commit message

`fix(maintenance): derive an enrichment-keyed ColumnScopedMerge cell for a clockless keyed model edge`
