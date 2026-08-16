# Phase 9 plan — delete-aware affected-key discovery

## Objective

Close the obligation-7 under-approximation phase 8 discovered: a key whose entire
window contribution is deleted from a `mutable_snapshot` source leaves no trace in
`repair_affected_keys_select`'s current-source scan, so its stale output row is never
repaired. Advances success criteria 1, 2 and 4 — the repair family's core promise
(a retraction repairs correctly) is not upheld until this closes, and the standing
conformance gate currently steers around it.

## Spec delta (spec-first — implement step makes these edits)

- `docs/specs/sources.md` §"The fingerprint sidecar" — the sidecar's partition grain is
  the *digested unit*, not necessarily one source row: a repair-family consumer
  partitions at **group** grain (one row per output group key, digest = an
  order-insensitive aggregate over that group's contributing rows). Same table, same
  stamp/invalidation rules; a group-grain partition is what makes a *deleted* unit
  observable, since only a stored comparandum can witness something that is gone.
- `docs/specs/incremental_models.md` §"The repair family" —
  - how obligation 7 discharges over a `mutable_snapshot` source with no native change
    feed: the group-grain sidecar diff *is* the affected-key relation;
  - the discovery read is a **full** source read, unbounded by the cell's `ScanClamp`
    (a clamped rescan against full stored digests flags every out-of-clamp group);
    the clamp still bounds nothing else it bounded before — the per-group recompute
    stays bounded by the key set, per obligation 4;
  - **absent or stale-stamped comparandum** ⇒ the affected set for that run is every
    currently-observed group *plus* every stored output group (a sound
    over-approximation that degenerates to a whole-table repair for one run, then
    self-heals on refresh). Distinguish this explicitly from the existing "never
    widens to a whole-table repair" sentence, which is about *admission* refusing an
    unprovable obligation, not about a runtime comparandum being missing.
  - Known Divergences: delete the
    `known_bug_repair_affected_key_discovery_misses_full_group_deletion` entry.

## Tests (red-green)

1. `emit::repair_group_digest_select_is_order_insensitive` (smelt-logical, real-DuckDB
   unit alongside `fingerprint_sidecar_tests`) — the same group's rows inserted in a
   different order digest identically; deleting one row of the group changes it.
2. `emit::repair_group_digest_diff_reports_a_vanished_group` — the sidecar diff over a
   group-grain partition returns a group present in the sidecar and absent from the
   source (the `__smelt_src.delta_key IS NULL` leg), with its key value intact.
3. `maintenance_driver::snapshot_source_discovery_uses_the_sidecar_diff` — for a
   `MutableSnapshot` delta posture the affected-key SQL is the emitter's diff, not the
   clamped `SELECT DISTINCT`; the append-only posture is unchanged in shape.
4. `maintenance_driver::snapshot_discovery_fails_loud_on_a_non_duckdb_backend` — the
   resolver returns `BackendError::unsupported` rather than silently using the unsound
   current-source scan (mirrors `diff_fingerprint_sidecar_changed_keys`'s precedent).
5. `repair_lowering::repair_fixes_a_key_whose_entire_window_contribution_was_deleted`
   — live DuckDB: seed two keys, delete every row of one, assert the stored output row
   for that key is gone/repaired and the other key is untouched. This is the gap case.
6. `repair_lowering::repair_covers_stored_output_keys_when_the_sidecar_is_absent` — drop
   the sidecar partition, assert the run still reaches a key that vanished while the
   comparandum was missing.
7. `maintenance_conformance/repair.rs::repair_pool_upholds_equivalence_under_retraction`
   — retarget the delete step at a key whose *entire* window contribution departs,
   deleting the workaround comment; the gate now covers what it previously dodged.
8. `registry::divergence_registry_staleness_report` — remove the
   `known_bug_repair_affected_key_discovery_misses_full_group_deletion` entry and its
   `known_bug_still_reproduces` arm (the structural literal it greps is gone).

## Tasks

1. Make the spec edits above.
2. Add `emit_repair_group_digest_select(source_table, group_key, digest_columns, dialect)`
   to `smelt-logical`'s `emit` — `GROUP BY` the group key, projecting the canonical
   `delta_key` expression `emit_fingerprint_digest_select` already builds plus an
   order-insensitive `delta_digest` aggregate over the per-row digests. DuckDB-scoped,
   same fail-loud posture as its per-row sibling.
3. Make the affected-key relation uniformly a one-column `delta_key` relation: rewrite
   `repair_affected_keys_select` (append-only path) to project the same canonical key
   expression, and join `emit_per_group_recompute`'s DELETE and candidate legs by that
   expression instead of by key columns. One shape, both paths — a deleted group's typed
   column values are unrecoverable by construction, so column-shaped joins cannot serve
   the snapshot path. Update `emit_diff_patch`'s `slice_predicate` caller accordingly.
4. Add the group-grain discovery path in `maintenance_driver.rs`: a repair-scoped
   partition identity (model + group-key columns + digest columns, so it never collides
   with a P4 per-row partition), reusing `emit_fingerprint_sidecar_diff`,
   `generate_fingerprint_sidecar_table_ddl` and the existing refresh DML verbatim.
5. Route it in `resolve_live_per_group_recompute_cell`: a `MutableSnapshot` delta posture
   takes the sidecar diff; anything else keeps the clamped scan. Non-DuckDB → `Err`.
6. Refresh the sidecar partition inside the repair's own statement group, after the write
   (the same-transaction shape `refresh_fingerprint_sidecar` already documents), and
   populate it on the create/full-refresh path so the first incremental run has a
   comparandum instead of taking the absent-comparandum degradation every time.
7. Implement the absent/stale-comparandum union with stored output keys.
8. Update the phase-8 conformance test and the registry per tests 7–8.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test statement_parity --test repair_lowering --test diagnostics`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-logical --test walk_coverage`

## Commit message

`fix(incremental): delete-aware affected-key discovery for repair over snapshot sources`
