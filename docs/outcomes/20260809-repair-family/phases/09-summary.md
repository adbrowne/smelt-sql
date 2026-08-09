# Phase 9 summary — delete-aware affected-key discovery

## Shipped

- Group-grain fingerprint sidecar: `emit_repair_group_digest_select` +
  `emit_repair_group_sidecar_diff` (`crates/smelt-logical/src/maintenance/emit.rs`) — one sidecar
  row per output group key, order-insensitive digest, reusing the existing per-row sidecar table
  and stamp/invalidation rules under a distinct `projection_identity` namespace.
- `emit_per_group_recompute` (same file) rewritten to join its DELETE/INSERT legs by the single
  canonical `delta_key` expression instead of raw key columns — a vanished group's typed values
  are unrecoverable, so both discovery paths (clamped scan, sidecar diff) now emit the same
  one-column affected-key relation shape. `repair_affected_keys_select`/`repair_candidate_select`/
  `repair_slice_predicate` (`crates/smelt-runtime/src/maintenance_driver.rs`) updated to match.
- `RepairDiscovery` verdict (`ClampedScan` / `SidecarDiff { digest_columns }`) threaded through
  `resolve_live_per_group_recompute_cell` (now takes a `dialect: SqlDialect`): a `MutationProfile::
  MutableSnapshot` source routes to the sidecar diff, everything else keeps the clamped scan, a
  non-DuckDB backend under `SidecarDiff` fails loud (`BackendError::unsupported`).
- `diff_repair_group_sidecar_changed_keys` / `refresh_repair_group_sidecar` (`maintenance_driver.rs`)
  — read/write sides of the group-grain sidecar, mirroring the per-row `diff_fingerprint_sidecar_
  changed_keys` / `refresh_fingerprint_sidecar` shape; absent/stale comparandum unions
  currently-observed keys with every stored output key (sound over-approximation, self-heals on
  refresh). `generate_fingerprint_sidecar_partition_exists_sql` (`crates/smelt-state/src/
  ddl_duckdb.rs`) added to distinguish "absent" from "stale".
- `execute.rs` routes affected-key construction by `RepairDiscovery` and seeds the sidecar's
  initial comparandum on a model's creation run so the first live repair isn't degraded.
- Spec: `docs/specs/incremental_models.md` §"The repair family" gained "Obligation 7 over a
  `mutable_snapshot` source"; `docs/specs/sources.md` §"The fingerprint sidecar" gained "Partition
  grain". Deleted the closed `known_bug_repair_affected_key_discovery_misses_full_group_deletion`
  Known Divergences entry and its registry `KnownBug` + staleness-check arm.
- 8 new/retargeted tests across `smelt-logical`, `smelt-state`, `smelt-runtime` (repair_lowering,
  statement_parity, diagnostics), and `smelt-cli` (maintenance_conformance/repair.rs + registry.rs).

## Decisions

- Group-grain partition identity is the SAME sidecar table, distinguished only by
  `projection_identity` text (`repair:group=<cols>:digest=<cols>`) — not a second mechanism or
  table, since the shape (key/digest/stamp/GC) is identical to the per-row sidecar.
- Digest columns are read off the cell's own already-derived `fingerprint_projections` rather than
  a second call to `delta_shape_for_source`/`fingerprint_projection` — one derivation, reused.
- Discovery read stays unbounded by the cell's `ScanClamp` (full source read for `MutableSnapshot`)
  per the spec delta's explicit ruling — a clamped rescan against full stored digests would flag
  every out-of-clamp group every run.

## For the next planner

- `emit_repair_group_digest_select`'s order-insensitive combiner (`bit_xor(hash(...))`) has a
  theoretical collision risk beyond the per-row sidecar's assumed SHA-256 soundness — worth a
  dedicated soundness note if this becomes load-bearing for a wider surface.
- Unconfirmed: whether a `grain: key` model reaching the snapshot-reconcile creation path (rather
  than window-forward) also needs the sidecar seed — only the window-forward branch was checked.
- No test exercises a *stale* (not absent) group-grain comparandum end-to-end; stale-path plumbing
  is only indirectly covered via the shared `has_stale` flag and the per-row sidecar's own tests.
- Phase 10 (decomposed-combiner hidden state) and phase 11 (`smelt explain` surface) are untouched,
  as scoped.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test statement_parity --test repair_lowering --test diagnostics`
  — 21 + 14 + (diagnostics passed as part of the same run) all green.
- `cargo test -p smelt-cli --test maintenance_conformance` — 58 passed.
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed.
