# Phase 6 plan — the three degraded routes, proved live on Trino

## Objective

Prove the routes Trino reaches *because* T3 declined every correctness structure: the repair
family's per-group recompute, the succession grain's ledger-less full rebuild standing in for the
window-forward patch, and the key-addressed model edge's sidecar-less downgrade. Each must be
recorded on the plan cell, visible in `smelt explain --json`, and oracle-equal to `--full-refresh`
on the live tier. Advances criteria 2 (per-group recompute executes), 3 (the unreachable families
degrade by name) and 8 (a downgraded cell is asserted oracle-equal, and the succession rebuild is
row- and column-identical to the ledger-bearing rebuild's presented arm).

## Spec delta

None. The routes are already specified — `docs/specs/state.md` §"The degradation contract" step 2
(the structure each technique requires, and `recompute_equivalent`'s fallback), and
`docs/specs/incremental_shapes.md` §"The succession grain" (the full rebuild replacing the patch).
Phase 2 already stated the Trino column of §"Incremental & schema evolution per backend". If a live
measurement contradicts any of those statements, the spec edit lands **first**, in the same commit.

## Tests

Live-gated (`SMELT_TRINO_URL`), red-green, in a new
`crates/smelt-cli/tests/trino_incremental_families/degraded_routes.rs` submodule unless noted.
Each must fail for a real reason before the fix, and skip green (never silently pass) when the
coordinator is absent — the existing `trino_env()` guard.

1. `per_group_recompute_cell_is_explain_visible_on_trino` — a repair-family cell (keyed
   non-invertible fold over a `mutable_snapshot`, clocked source; shape per
   `crates/smelt-runtime/tests/repair_lowering.rs`) reports `Technique::PerGroupRecompute` in
   `smelt explain --json` on a `trino` target, with its `scans` clamp present.
2. `per_group_recompute_matches_full_refresh_on_trino` — that model runs end-to-end through
   `smelt run --target trino`, and after a genuine in-place source mutation its table is
   row-identical to a `--full-refresh` oracle schema.
3. `succession_cell_records_state_downgraded_on_trino` — a `grain: succession` model on a `trino`
   target carries `state_downgrade { original: SuccessionPatch, missing: <tombstone ledger> }`
   in explain JSON, and its run never demands `--event-time-start/--event-time-end`.
4. `succession_downgraded_rebuild_matches_ledger_bearing_presented_arm` — the presented table the
   Trino downgraded run writes is row- and column-identical (column names, order, and the full row
   multiset) to the presented table a DuckDB run of the same fixture writes through the
   ledger-bearing `rebuild_succession_state` arm. Criterion 8's second sentence, measured rather
   than argued.
5. `sidecar_less_key_addressed_cell_downgrades_on_trino` — a key-addressed upstream-model-edge cell
   (`KeyScope`/`KeyDiscovery::UpstreamKeyed`, requiring `StateStructure::FingerprintSidecar`)
   reports `state_downgrade { original: PerGroupRecompute, missing: FingerprintSidecar }` and
   `technique: DeleteInsert` in explain JSON on a `trino` target.
6. `key_addressed_downgrade_matches_full_refresh_on_trino` — that model runs live and is
   oracle-equal after an upstream model's rows change; in particular the run must **not** dispatch
   `maintenance_driver::key_addressed` (which needs the sidecar) for the downgraded cell. If it
   still does, the fix is a dispatch-side guard read off `cell.state_downgrade`, never off
   `backend.dialect()` (`tests/state_guard_census.rs`'s discipline).

## Tasks

1. Stage the repair-family fixture over declared sources (phase 4's rule: a between-run mutation
   needs a declared source, never an inline `materialization: table` model), seeded directly on the
   live tier; write tests 1-2 red.
2. Make tests 1-2 green. Expect no emitter change: phase 5's `changed_row_delete` helper already
   covers `emit_per_group_recompute`'s `DELETE` leg on Trino.
3. Stage the succession fixture; write tests 3-4 red, with the DuckDB comparison arm running the
   same project against a `duckdb` target in the same test.
4. Make tests 3-4 green. `rebuild_succession_state`'s `cell.state_downgraded` arm and
   `emit_succession_full_rebuild_ledgerless` already exist — this is a live proof, plus whatever
   plan-to-dispatch wiring the measurement shows missing.
5. Stage the key-addressed upstream-model-edge fixture; write tests 5-6 red.
6. Make tests 5-6 green, guarding the key-addressed dispatch on the plan's own downgrade verdict if
   the measurement shows it dispatching a sidecar-dependent route.
7. Record every measured live error text (Trino's own message) in the phase summary and, where it
   contradicts a spec statement, in the Decision log.
8. Keep each new file under the large-file cap (`.claude/scripts/large-file-check.sh`); split the
   submodule rather than registering an exception, as phase 5 did.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN.
- `cargo test -p smelt-runtime --test state_guard_census --test availability_seam --test execute_parity`
- `cargo test -p smelt-logical --test maintenance_availability --test walk_coverage`
- `cargo test -p smelt-cli --test trino_explain_downgrade --test trino_ci_wiring --test trino_incremental_spec_freshness`
- Live tier (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`; `bash scripts/trino-down.sh`):
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1`.
  If the coordinator cannot be brought up, emit `<<PHASE_BLOCKED>>` — never report a skipped live
  leg as a pass.

## Commit message

`feat(trino): prove the per-group-recompute, succession-rebuild and sidecar-less key-addressed degraded routes live`
