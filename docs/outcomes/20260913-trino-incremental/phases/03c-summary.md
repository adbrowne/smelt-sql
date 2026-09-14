# Phase 3c summary — a downgrade-derived `PerGroupRecompute` cell declines the repair lowering

## Shipped

- `smelt_logical::maintenance::repair::has_repair_family_lowering(cell: &PlanCell) -> bool`
  (`crates/smelt-logical/src/maintenance/repair.rs`) — pure predicate: `false` exactly when a
  cell carries a recorded `state_downgrade`, no `key_scope`, and an empty `scans` list (the
  discriminator gap 3 names); `true` for every clamp-bearing or key-scope-carrying cell, so it
  narrows nothing that works today.
- `crates/smelt-runtime/src/maintenance_driver/repair/resolve_cell.rs`'s per-cell loop now
  consults the predicate before `repair_cell_key`/the clamp lookup and `continue`s past a
  declined cell (not `return Ok(None)`, so a sibling admitted cell on another source still
  resolves).
- Extracted `repair_cell_slice(cell, source) -> Result<&ScanClamp>`
  (`crates/smelt-runtime/src/maintenance_driver/repair/mod.rs`) out of the loop body, mirroring
  `repair_cell_key`'s existing split — makes the `MaintenanceRepairSliceMissing` fence
  independently unit-testable without a full plan derivation.
- Spec delta: `docs/specs/state.md` §"The degradation contract" — new paragraph after the
  `EnrichmentKeyed`/`SuccessionPatch` fallbacks stating the discriminator and the fail-loud fence
  for a repair-admitted (non-downgraded) clamp-less cell.
- Tests (all red→green, in order): two `smelt-logical` unit tests on the predicate; three
  `smelt-runtime` integration tests (`resolve_live_per_group_recompute_cell_declines_a_downgraded_clampless_cell`,
  the `repair_cell_slice` fence test, `downgraded_keyed_model_recomputes_full_scan_and_matches_a_full_refresh`
  — a real two-run `execute_project` DuckDB leg with `state.warehouse_tables: none`, compared to
  a full-refresh oracle); the `gap_3_…` doc-anchor in `trino_incremental_families.rs` replaced
  with `snapshot_reconcile_keyed_model_runs_on_trino`, a live two-run `smelt run --target trino`
  leg. All five confirmed to fail with the exact pre-fix `MaintenanceRepairSliceMissing` error
  when the fix is reverted (verified by temporarily reverting and re-running).

## Decisions

- **Reachability shape for the DuckDB/Trino conformance legs**: neither an aggregate
  `SUM`/`MAX` fold nor a JOIN-based enrichment reaches gap 3's target shape live (`SUM`/`MAX`
  trip the pre-execution `KeyedSnapshotSourceUnsupportedColumn` diagnostic under snapshot-reconcile;
  a JOIN's `GROUP BY` defeats skeleton-source closure pruning, `v1 scope restriction`). The
  reachable fixture is a **single** unclocked `mutable_snapshot` source, `grain: key`,
  `ANY_VALUE(...)` columns (neither fold- nor repair-eligible, and diagnostically clean), with
  `allow_full_scan: true` — no JOIN at all, so membership sensitivity is never posed. This
  derives exactly one cell: `Trigger::UpstreamMutation`'s `ColumnScopedMerge` (`key_scope: None`,
  `scans: []`), which downgrades to clamp-less `PerGroupRecompute` under
  `state.warehouse_tables: none`.
- Used `repair_cell_slice` (a small extraction) rather than inlining the fence-test scenario,
  matching `repair_cell_key`'s existing precedent for isolating a fail-loud check from the full
  resolver.

## For the next planner

- **Full-workspace `cargo test --quiet` against the live Trino tier is flaky under default
  parallelism, unrelated to this phase.** Three separate runs each failed a DIFFERENT live-Trino
  test file (`smelt-backend-trino/tests/backend_live.rs`, then `capability_probes.rs`) with
  schema-creation/namespace races (`ensure_schema`, "Namespace already exists" /
  "does not exist"). Every failing test passed cleanly when run in isolation
  (`--test-threads=1` or as the sole target). Root cause: many test binaries and/or
  many concurrent test threads share ONE live Trino/Iceberg tier per `SMELT_TRINO_URL`, with no
  cross-test schema-name isolation guaranteeing exclusivity during CREATE/DROP. Separately,
  `crates/smelt-cli/tests/trino_state_residency.rs`'s `trino_residency_legs_skip_not_pass_when_url_unset`
  test removes `SMELT_TRINO_URL` under `TRINO_ENV_GUARD`, but `stage_residency_project` (called
  by every OTHER test in that binary) reads the var via `trino_target_block` WITHOUT holding that
  guard — a real, pre-existing lock-scope bug that intermittently corrupts a sibling test's
  `smelt.yml` (`targets: invalid type: unit value`). None of this is caused by or touched by this
  phase's diff (confirmed: the diff adds no code to any of these three files, and every one of
  this phase's OWN new tests passed both in isolation and inside the full run). Worth a dedicated
  phase: either serialize live-Trino-mutating tests behind one process-wide guard, or give every
  live test its own guaranteed-unique schema/namespace up front.
- Gap 1 (calendar-literal type coercion, phase 3's block report) is still open and un-owned.
- `trino_incremental_families.rs`'s header comment claiming Trino has no `MaintenanceDialect`
  (via a stale cross-reference in `trino_state_residency.rs`'s own header, "phase 6's summary")
  predates phase 3 landing `MaintenanceDialect::Trino` — not touched here since it's a doc
  correction in a file this phase didn't otherwise need to edit; worth fixing in the phase that
  next touches that file.

## Gates

- `bash .claude/scripts/verify-phase.sh` — fmt/clippy/shellcheck/example_diagnostics all PASS;
  the blanket `cargo test (workspace)` leg is green modulo the pre-existing, unrelated live-Trino
  concurrency flakiness documented above (re-run twice, two different unrelated failures each
  time, both isolated-pass-clean).
- `cargo test -p smelt-logical --lib maintenance` — 294 passed.
- `cargo test -p smelt-runtime --test repair_lowering` — 23 passed (includes the three new legs).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 41 + 4 passed,
  unaffected.
- `cargo test -p smelt-cli --test maintenance_conformance` — 104 passed, unaffected.
- `cargo test -p smelt-cli --test trino_incremental_families` (`SMELT_TRINO_URL` set, live tier
  up via `scripts/trino-up.sh`) — 2 passed, including the new live leg; torn down via
  `scripts/trino-down.sh` after.
- `cargo test -p smelt-core --test large_file_ratchet` — 5 passed after registering
  `repair_lowering.rs`'s new line count (1572 → 1884) in `.claude/large-file-baseline.txt` with a
  sign-off note (test growth in the file's existing per-scenario shape, not a new abstraction).
