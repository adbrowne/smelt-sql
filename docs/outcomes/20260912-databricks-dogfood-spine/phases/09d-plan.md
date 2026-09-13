# Phase 9d plan — replay the Databricks state under 9c's fix and re-measure both sweeps

## Objective

The committed `08-parity.json` and `09b-equivalence.json` were measured against the pre-fix
dispatch that 9c root-caused, so both are stale for `silver_actor_naming` and neither ratchet can
be restored on top of them. This phase rebuilds the Databricks dogfood state from scratch under
the fixed binary — load days 1–8, full refresh, then windows 9/10/11 each with its oracle refresh
— re-runs the dual-target parity sweep and the equivalence sweep over that state, commits both
refreshed reports, and restores the deferred liveness ratchet. Closes criterion 7 and re-closes
criterion 8 on live numbers (rows 8 and 9b come off `## Blocked`).

**Live phase.** If `scripts/dbx-dogfood-env.sh` + `scripts/dbx-auth.sh` cannot reach the
workspace, stop and emit `<<PHASE_BLOCKED>>` — never skip green.

## Spec delta

`docs/specs/state.md` §"The degradation contract" (the numbered downgrade rules around the
`DeleteInsert` full-region-recompute fallback): add one sentence stating that for a
**succession-grain** cell the recompute region is the whole presented table, so on a backend with
no realisable `TombstoneLedger` every run rebuilds the model in full from the whole source seen so
far — the cost the contract trades for correctness, and the reason the downgraded cell must not
take the window-forward patch route. 9c landed the behaviour; this records it. No user-facing
surface change, so no `docs-site/` edit here (phase 10 owns the Databricks page).

## Tests

- `oracle_driver_declares_a_reset_stage_scoped_to_the_dogfood_schemas` (offline, in
  `crates/smelt-cli/tests/github_activity_dbx_oracle.rs`) — the new `reset` stage's statement list
  names only `workspace.smelt_dogfood` / `workspace.smelt_dogfood_oracle` objects, and truncates
  rather than drops `github_events`; it can never name another catalog or schema.
- `the_parity_driver_declares_the_checkpoint_the_sweep_expects` (offline, in
  `github_activity_dual_target.rs`) — the parity script's day count and checkpoint defaults
  (now 11 days, single checkpoint at day 11) are in lockstep with the constant the sweep reads,
  mirroring the oracle suite's existing schedule-lockstep test.
- `dbx_registry_entries_are_all_live` (offline, restored) — every entry of
  `TARGET_DIVERGENCE_REGISTRY`'s Databricks half is witnessed by a difference in the committed
  `08-parity.json`, and every difference in that report is registered. Both directions.
- `the_committed_parity_report_shows_no_unregistered_difference` (offline) — the report-driven
  gate over the refreshed `08-parity.json`, twin of the oracle suite's
  `the_committed_equivalence_report_shows_no_violation`.
- `the_committed_equivalence_report_shows_no_violation` (offline, already present) — must go
  green against the refreshed `09b-equivalence.json` with no new registry entry; if it needs one,
  that is a finding to record, not a bound to invent.
- `duckdb_and_databricks_agree_on_every_model` (live) and
  `databricks_incremental_matches_its_oracle_at_every_window` (live) — the two sweeps, re-run.

## Tasks

1. Land the `state.md` spec delta.
2. Add a `reset` stage to `scripts/dbx-dogfood-oracle.sh`: drop every model table/view in
   `workspace.smelt_dogfood` and `workspace.smelt_dogfood_oracle`, `TRUNCATE TABLE` the source
   `workspace.smelt_dogfood.github_events` (never `DROP` — the source declaration is the
   contract), and remove the local `.smelt/targets/` state for the `databricks` and
   `databricks_oracle` targets. Document it as the one destructive stage, with the
   BigQuery driver's destructive stage as precedent. Red-green against the new offline test.
3. Update `scripts/dbx-dogfood-parity.sh`'s day/checkpoint defaults to the 11-day end state the
   replay now produces, and the sweep-side constant with it; red-green against the new lockstep
   test.
4. **Live replay**, in one sequence under the freshly built binary: `source
   scripts/dbx-dogfood-env.sh`; `bash scripts/dbx-auth.sh`; `reset`; land fixture days 1–8 with
   `scripts/dbx-dogfood-loader.py --date …`; `smelt run --target databricks --full-refresh
   --allow-full-refresh` over `[2026-08-05, 2026-08-13)`. Sanity check before continuing: the
   16 model row counts must match phase 6/7b's recorded 8-day counts — a mismatch means the
   replay is not reproducing the original history and must be reported, not papered over.
5. For n in 9, 10, 11: `dbx-dogfood-oracle.sh window n`, `oracle n`, `snapshot n`. Refresh the
   token between checkpoints if the hour lapses.
6. Equivalence leg: `oracle-sh manifest`, then the live sweep with
   `EQUIVALENCE_REPORT_OUT=phases/09b-equivalence.json`, then `report`. Commit the refreshed
   JSON and its markdown twin. `silver_actor_naming` is expected to match exactly at all three
   checkpoints (it already did pre-fix, on the oracle leg).
7. Parity leg: `dbx-dogfood-parity.sh duck` (11 days), `dbx-snapshot`, `manifest`, the live
   sweep writing `phases/08-parity.json`, then `report`. `silver_actor_naming`'s
   `databricks_only=520` must now be `0`; if it is not, the fix is incomplete — record the
   measurement and block rather than registering a bound.
8. Restore `dbx_registry_entries_are_all_live` and the report-driven parity gate, and delete the
   "no committed report or liveness ratchet yet" deferral notes from
   `github_activity_dual_target.rs`'s and `parity_support`'s module doc comments.
9. Flip rows 8 and 9b to `done` in the phase table; mark both `## Blocked` entries RESOLVED by 9d
   (keeping their text, as the 9c entry does); append the dated Decision log entry with the
   measured before/after for `silver_actor_naming`.
10. Write `phases/09d-summary.md`: the measured tables, the replay sequence actually run, any
    Free-Edition quota or cold-start cost worth adding to `free-edition-facts.md`, and what phase
    10's findings handoff should carry from here.

## Verification

- `bash .claude/scripts/verify-phase.sh` — the standing gate.
- `cargo test -p smelt-cli --test github_activity_dual_target` — offline legs, with the restored
  ratchet green against the refreshed report.
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — offline legs green against the
  refreshed report.
- The two live sweeps above, each with `SMELT_DBX_DOGFOOD_LIVE=1`.
- `cargo test -p smelt-runtime --test statement_parity` and `cargo test -p smelt-cli --test
  maintenance_conformance` — unchanged by this phase; run to confirm the replay work touched
  nothing they own.

## Commit message

`outcome(databricks-dogfood-spine): phase 9d replays Databricks under the succession fix and re-measures both sweeps`
