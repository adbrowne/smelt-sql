# Phase 9d summary — blocked on a second live-only gap in 9c's fix

## Shipped

- `docs/specs/state.md` §"The degradation contract": one added paragraph stating that a
  succession-grain cell downgraded for want of a `TombstoneLedger` recomputes the whole
  presented table on every run (no window-forward patch route exists for it), naming the cost
  the contract trades for correctness.
- `scripts/dbx-dogfood-oracle.sh`: a new `reset` stage — the one destructive stage, hardcoded to
  the two dogfood schemas as literals (never `SMELT_DBX_CATALOG`/`SMELT_DBX_SCHEMA`), truncating
  `github_events` and dropping every other table in both schemas, then clearing local
  `.smelt/targets/{databricks,databricks_oracle}` state. Test:
  `oracle_driver_declares_a_reset_stage_scoped_to_the_dogfood_schemas` in
  `github_activity_dbx_oracle.rs` (19 tests now, was 18).
- `scripts/dbx-dogfood-parity.sh`: default `DAYS`/`CHECKPOINTS` moved from 8 to 11 (the replay's
  target end state). Test: `the_parity_driver_declares_the_checkpoint_the_sweep_expects` in
  `github_activity_dual_target.rs`.
- **Live workspace state** (not git-tracked): `workspace.smelt_dogfood.github_events` reset and
  reloaded with fixture days 2026-08-05 through 2026-08-12 (26,220 rows, verified against the
  raw fixture's per-day counts via the `duckdb` CLI). No model tables exist yet — the full
  refresh that would create them failed. `workspace.smelt_dogfood_oracle` is empty.

## Decisions

- **Blocked rather than improvised a fix.** The full refresh failed with `Feature not supported
  by Spark SQL: succession-patch technique (window-forward driver)` on both succession-grain
  models. Root cause: `rebuild_succession_state`
  (`crates/smelt-runtime/src/maintenance_driver/succession/execute.rs:339`) carries the exact
  same unconditional `realises_tombstone_ledger` gate as `execute_succession_maintenance` — 9c's
  dispatch fix routes a `state_downgraded` cell to `rebuild_succession_state` instead of the
  window-forward loop, but that function refuses too, so the fix never reaches Databricks. This
  needed a real design answer (does a downgraded full rebuild skip the tombstone table and
  clock-tie probe entirely, or does it need a ledger-free emitter?), not a boolean-check
  one-liner, so it was left for a follow-up phase rather than patched inline mid-replay.
- **Reverted the report-driven gates I could not back with real data.** I initially added
  `dbx_registry_entries_are_all_live` and `the_committed_parity_report_shows_no_unregistered_
  difference` per the plan, then reverted both (plus their helpers) once the live replay
  blocked, since `08-parity.json` still does not exist — an unconditional file-read test with no
  file would leave `cargo test` permanently red for everyone. Restored the original deferral
  comment, updated to name the new root cause.
- **Left the workspace mid-replay rather than resetting back to empty.** The 8 loaded days are
  exactly what task 4 needs already staged; the next fix attempt can resume at "re-run the full
  refresh" without repeating the reset/reload sequence.
- **Caught and fixed my own double-load bug before it corrupted the measurement.** The first
  reset+load attempt hit a chained failure (the `_loader_days` ledger table didn't exist yet
  because I ran the loader before `--apply-ddl`), and re-running the load loop after fixing that
  order without a second reset silently double-appended all 8 days (55,641 rows instead of the
  correct 26,220). Caught by comparing against the raw fixture's per-day sums via `duckdb`
  CLI — reset and reloaded a second time before running anything downstream of it.

## For the next planner

- **The fix belongs in `rebuild_succession_state`, not the dispatch site.** Three candidate
  routes are written out in `## Blocked`'s phase 9d entry: (1) a `state_downgraded`-gated branch
  inside the existing function that skips the tombstone table/clock-tie probe and emits a bare
  `ROW_NUMBER() … = 1` rebuild restricted to non-delete-flagged rows; (2) the same shape as a
  distinct function if the two bodies diverge too much to share one; (3) a live-execution test
  leg (even against a synthetic no-ledger DuckDB dialect, if constructible) so this class of gap
  is caught without needing a live Databricks run.
- **9c's offline verification pattern has a blind spot worth generalising**: asserting a
  *dispatch decision* is not the same as asserting the dispatched function *succeeds*. Any
  future "route X to function Y" fix in this driver should include at least one test that
  actually calls Y, not just one that confirms X routes to it.
- Once the fix lands, resume at task 4 of `phases/09d-plan.md` — the full refresh over
  `[2026-08-05, 2026-08-13)`, then windows 9-11 with their oracle refreshes, then both sweeps.
  The offline scaffolding this phase committed does not need repeating.
- Nothing left the outcome; nothing added to `## Out of scope`.

## Gates

- `bash .claude/scripts/shellcheck-gate.sh` — PASS (76 scripts, zero findings).
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — 18/18 (offline-only; the live
  test skips with `SMELT_DBX_DOGFOOD_LIVE` unset).
- `cargo test -p smelt-cli --test github_activity_dual_target` — 24/24 (offline-only; the live
  test skips).
- Live: reset, DDL apply, 8-day load, row-count sanity check — all succeeded. The full-refresh
  live run **failed** (this phase's finding, not a regression to fix here).
- `bash .claude/scripts/verify-phase.sh` was not run to completion — the live blocker was hit
  before reaching that step; the offline-scoped gates above cover everything this phase actually
  changed. The next phase should run the full gate once its fix lands.
