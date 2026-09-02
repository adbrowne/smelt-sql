# Phase 24b plan — bare-keyed→reader model-edge admission (`RepairKeysNotDiscoverable`)

## Objective

`smelt explain silver.device_user_edges` refuses with
`RepairKeysNotDiscoverable { source: "silver.events_deduped", why: "grain expression reads
column 'device_id' absent from the delta's own row shape" }`, so the last maintained
`examples/web_analytics` model is silently unscheduled. Admit the case where the downstream's
grain columns are **columns of the upstream relation** rather than the upstream's own
`KeyedUpsert` key columns, discovering the affected key set by grouping the sidecar at the
downstream's grain over the upstream table. Serves success criterion 15 (`examples/web_analytics`
fully `--since-upstream`-compatible; "Graph-layer gaps" bullet removed).

## Spec delta (spec-first — the implement step makes this edit before code)

`docs/specs/incremental_models.md` §"Upstream model edges", the two paragraphs beginning
"An upstream maintained model whose own derived delta signature is `keyed upsert`" and "A
key-addressed cell's affected-key set is discovered from the **group-grain fingerprint sidecar
diff**":

1. **Admission leg corrected.** Today's text refuses "when the downstream's own SQL does not
   carry the upstream's key columns". That is tighter than the mechanism needs: the discovery
   reads the *upstream relation*, so the real obligation is that the downstream's grain columns
   all resolve to columns of that upstream, reached through a plain single-relation `FROM` with
   no fan-out join. Restate the refusal in those terms (grain not resolvable **against the
   upstream relation**, or a fan-out join between them).
2. **Discovery keyed at the downstream's grain when the two key sets differ.** Today's text
   pins the sidecar at the upstream's key columns and then projects changed upstream keys
   forward (`SELECT DISTINCT <downstream key> FROM <upstream> WHERE <upstream key> IN (…)`).
   That projection reads the *post-change* upstream, so a row whose grain value **moved**
   between downstream groups surfaces the new group and never the old one — an
   under-approximation the equivalence invariant forbids. Correct it: when the downstream's
   grain differs from the upstream's key, the sidecar is grouped at the **downstream's grain
   projected over the upstream relation**, and the diff's own `delta_key` *is* the affected
   downstream key (both the vacated and the arriving group flip their order-insensitive XOR
   digest, so both surface). The existing upstream-keyed route stays verbatim for the
   equal-key case, where no move is representable. State this as two named discovery routes.
3. Remove §Known Divergences' "**Graph-layer gaps**" bullet — verify all three clauses first
   (bare `grain: key` nodes past `MaintenanceGraphUnsupportedNode` and key-level dirt were
   phases 21/22; whole-workspace `--since-upstream` is 24 + this phase). If a clause survives,
   narrow the bullet to it instead of deleting.

## Tests (red → green)

- `crates/smelt-logical/tests/keyed_model_edge.rs::grain_over_upstream_columns_is_admitted` —
  downstream `SELECT device_id, user_id, COUNT(*) … FROM smelt.silver.agg GROUP BY device_id,
  user_id` over an edge keyed `[event_id]` admits a cell whose `key_scope.keys ==
  [device_id, user_id]` and whose discovery route is the grain-over-upstream one.
- `…::grain_from_another_relation_is_still_refused` — the downstream's grain column comes from a
  second relation (joined), not the keyed upstream: still `RepairKeysNotDiscoverable`.
- `…::fan_out_join_blocks_the_grain_route` — a fan-out join between downstream and upstream
  refuses by name rather than admitting a projection that isn't the downstream's grain source.
- `…::consumer_not_carrying_upstream_keys_is_refused` (existing) — retarget: its downstream
  reads `order_id` which the upstream does *not* carry, so it must stay refused under the new
  admission leg. Adjust the fixture if it accidentally became admissible.
- `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs::grain_route_groups_sidecar_at_downstream_grain`
  — `resolve_live_key_addressed_model_edge_cell` returns the downstream grain as the sidecar
  group key (not the upstream key) and no longer bails `MaintenanceKeyScopeColumnMissing`.
- `…::moved_grain_value_repairs_both_groups` — live DuckDB: an upstream row whose grain column
  moves from group A to B causes both A and B to be recomputed (this is the leg the old
  post-state projection would have missed).
- `…::equal_key_route_is_unchanged` — the upstream-keyed route still emits
  `emit_key_addressed_affected_keys_select`'s shape byte-for-byte.
- `crates/smelt-cli/tests/explain_maintenance.rs::device_user_edges_admits_a_key_addressed_cell`
  — real `examples/web_analytics`: `smelt explain silver.device_user_edges` reports the
  key-addressed cell and no `RepairKeysNotDiscoverable`.
- `crates/smelt-cli/tests/since_upstream.rs` — extend phase 24's whole-workspace `--dry-run`
  flagship to assert `silver.device_user_edges` now appears in the RUN set.

## Tasks

1. Make the spec edits above (spec-first), including the divergence-bullet decision.
2. `KeyScope` (`crates/smelt-logical/src/maintenance/mod.rs`) gains a `discovery` field —
   `KeyDiscovery::{UpstreamKeyed, DownstreamGrainOverUpstream}` — so the route is plan data,
   not re-derived by the runtime (maintenance-plan purity). Update every construction site.
3. `admit_key_addressed_recompute` (`maintenance/repair.rs`): keep route 1 (delta columns =
   `edge_keys`) first; on `NotDiscoverable`, attempt route 2 with a delta shape whose columns
   are `fingerprint_projection(sql, edge_name)`'s `Columns` (the downstream-read columns of the
   upstream); gate route 2 on `model_property_vector(sql, join).has_fan_out_join == false`
   explicitly, since `resolve_grain` short-circuits the fan-out gate on a declared
   `unique_key`. `Projection::FullRow` ⇒ no route 2 (fail closed, as today).
4. `resolve_live_key_addressed_model_edge_cell` (`smelt-runtime/src/maintenance_driver.rs`):
   replace the unconditional `key_scope.keys ⊆ upstream_keys` bail with a match on
   `discovery` — the subset check stays for `UpstreamKeyed`; for
   `DownstreamGrainOverUpstream` the returned sidecar group key becomes `key_scope.keys` and
   the caller skips the forward projection, using the diff's `delta_key` directly.
5. Thread the route through `execute.rs`'s call site and the affected-key relation builder;
   `smelt explain` (`smelt-cli/src/explain.rs`) names which route a cell took.
6. Re-run `/smelt:validate incremental_models` on the §"Upstream model edges" and
   "Graph-layer gaps" text only; remove/narrow the bullet per task 1.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test keyed_model_edge`
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering`
- `cargo test -p smelt-cli --features duckdb --test explain_maintenance --test since_upstream`
- `cargo test -p smelt-cli --test maintenance_conformance` (equivalence invariant — the gate
  that would catch an under-approximating discovery route)
- `cargo test -p smelt-runtime --test statement_parity` and
  `cargo test -p smelt-logical --test walk_coverage`

## Commit message

`feat(maintenance): admit a key-addressed model edge whose grain lives on the upstream relation`
