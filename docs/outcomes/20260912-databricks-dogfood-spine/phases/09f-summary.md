# Phase 9f summary — closes criteria 7 and 8 on post-fix live numbers

## Shipped

- A clean **16/16** full refresh over `[2026-08-05, 2026-08-13)` on Databricks, under 9e's
  `emit_succession_full_rebuild_ledgerless` fix. `silver.actor_naming`: 25,700 rows (exact match
  to DuckDB; was 26,177 with up to 7x row duplication pre-fix).
- Three consecutive incremental windows (`2026-08-13/14/15`), each 16/16, each matching its own
  oracle full-refresh exactly at every relation.
- Refreshed `phases/09b-equivalence.json` / `09b-equivalence.md`: `silver_actor_naming` exact at
  all three checkpoints; only `gold_events_enriched` diverges (its already-registered
  `UnorderedColumnDivergence`).
- Refreshed `phases/08-parity.json` / `08-parity.md`: an 11-day DuckDB replay vs. the Databricks
  end state. `silver_actor_naming`'s `dbx_only`: **0** (was 520). Only `gold_events_enriched`
  still diverges, identically bounded.
- Two restored gates in `crates/smelt-cli/tests/github_activity_dual_target.rs`:
  `the_committed_parity_report_shows_no_unregistered_difference` and
  `dbx_registry_entries_are_all_live`, plus `dbx_parity_report`/`dbx_divergent_relations` helpers
  — 26/26 offline, up from 24.
- Rows 8, 9b, 9d flipped to `done`; their `## Blocked` entries marked RESOLVED (text kept).

## Decisions

- **Cleared stale local `dev` target state before the DuckDB parity replay.** An earlier
  session's `examples/github_activity/.smelt/targets/dev` + `target/dev.duckdb` left a ledger
  disagreeing with a fresh reload of day 2026-08-05, tripping `SourceMutationProfileViolated`.
  Removed (both gitignored, no tracked state lost) rather than working around it — same
  precedent as 9b's summary.
- **Task 2's staging check tolerated leftover model tables.** The plan expected "no model
  tables"; instead `bronze_events`/`github_events_arrival`/`silver_events_deduped` existed from
  9d's aborted full-refresh attempt. Source table row count and date range matched exactly
  (26,220 rows, 2026-08-05..08-12), so proceeded rather than resetting — the full refresh
  overwrites all model tables anyway.
- **No new divergence, no new registry entry.** The sweep is clean beyond the one already-known,
  already-registered `gold_events_enriched` bound; nothing new to register or triage.

## For the next planner

- **9c → 9e's lesson generalises beyond this one fix**: a test that asserts a dispatch
  *decision* is not a test that the dispatched function *succeeds*. Worth a note in any future
  maintenance-driver work.
- Criteria 7 and 8 are now closed on measured, live, post-fix numbers. Row 10 (findings handoff)
  can now cite this phase's `08-parity.json`/`09b-equivalence.json` as the final, trustworthy
  evidence rather than a stale pre-fix snapshot.
- Nothing left the outcome; nothing added to `## Out of scope`.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS.
- `cargo test -p smelt-cli --test github_activity_dual_target` — 26/26 (both restored report
  gates green).
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — 18/18.
- Two live sweeps (`databricks_incremental_matches_its_oracle_at_every_window`,
  `duckdb_and_databricks_agree_on_every_model`), each `SMELT_DBX_DOGFOOD_LIVE=1` — both PASS.
- `cargo test -p smelt-runtime --test succession_downgraded_rebuild --test statement_parity` and
  `cargo test -p smelt-cli --test maintenance_conformance` — all green, untouched by this phase.
