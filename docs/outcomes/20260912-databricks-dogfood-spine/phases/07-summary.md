# Phase 7 summary — three incremental windows on Databricks

## Shipped

- **Three consecutive incremental windows landed and run to completion**, each loading one real
  fixture day and driving `smelt run --target databricks --event-time-start … --event-time-end
  …`:
  - W1 (`2026-08-07`, run `20260912-132231-076699`): 14 success / 1 failed / 1 skipped.
  - W2 (`2026-08-08`, run `20260912-132430-360bcc`): 14 success / 1 failed / 1 skipped.
  - W3 (`2026-08-09`, run `20260912-132551-6435fe`): 14 success / 1 failed / 1 skipped.
  Reports at `examples/github_activity/.smelt/targets/databricks/reports/<run_id>.json`.
- **Frontier advanced correctly and selectively.** `intervals.json` for every successful model
  moved `2026-08-05→2026-08-07` → `→2026-08-08` → `→2026-08-09` → `→2026-08-10`;
  `gold.events_enriched` (the known failure) stayed pinned at `2026-08-05→2026-08-07` across all
  three windows — the frontier does not silently advance past a failed model.
- **Row-count oracle confirmed both raw landings.** `github_events`/`github_events_arrival` grew
  by exactly 2,388 rows for the `2026-08-07` load (DuckDB's own `--emit-slice-sql` UNION ALL
  query independently computes 2,388), and `silver.events_deduped` grew by exactly 2,334 —
  2,388 minus the ~54-row reach-back duplicate the dedup layer is supposed to drop. The
  event-time-only slice a naive per-day filter would compute (2,334) undercounts the true
  landing by the reach-back rows, which is why the phase 7 plan's own count check (task 2) had
  to be done as a before/after table delta, not a `WHERE created_at = …` filter.
- **Engine-resident state inventoried against the spec.** `SHOW TABLES IN
  workspace.smelt_dogfood` is the same 19 tables before and after all three windows — no
  ledger, sidecar, or tombstone table exists on this target, matching `docs/specs/state.md`
  §"Which dialects realise which structure" (all five listed structures are "no" for Spark/
  Delta). Every window's bookkeeping lives entirely in `.smelt/targets/databricks/*.json`
  (`intervals.json`, `landed_deltas.json`, the run reports) rather than in the warehouse.
- Recorded incremental-window wall time and a run-report gap in
  `docs/outcomes/20260912-databricks-dogfood-spine/free-edition-facts.md`.

## Decisions

- Treated the loader's before/after table-count delta as the count oracle instead of a
  per-day `WHERE` filter, once the filter undercounted by the reach-back rows (54 of 2,388) —
  the filter only sees the query's first `UNION ALL` arm.
- Did not attempt to fix `gold.events_enriched`'s failure or `marts.star_growth`'s skip; carried
  forward exactly as the plan specified, since 14/16 still completes and row 7b owns the gap.

## For the next planner

- **Run report `completed_at`/`duration_ms` are dead fields.** All three reports show
  `completed_at: null`, `duration_ms: 0` despite the run finishing and printing a result. This
  is orthogonal to phase 7's scope (no production code edited this phase) but is a real gap for
  row 9's oracle comparison or any future timing-based assertion — worth a small fix, not
  discovered before now because no prior phase read these fields.
- **The one-hour OAuth token expired mid-session** (`scripts/dbx-auth.sh` re-run was required
  once, between the baseline read and W1) — this is the same constraint `free-edition-facts.md`
  and outcome.md's "## Blocked" item (b) already flag for headless phases 5–9/11; phase 7
  reconfirms it rather than discovering something new. No action taken beyond re-minting.
- Row 7b's scope (from the outcome table) is unchanged by this phase: give
  `gold.events_enriched` a realisable route on Delta (sidecar realisation or a plan-derivation
  downgrade). This phase adds no new evidence beyond confirming the failure recurs identically
  in all three incremental windows, exactly as the row 6f baseline predicted.
- Row 8 (parity) and row 9 (oracle) can now read three real run reports plus the frontier/table
  state captured here.

## Gates

- `bash .claude/scripts/verify-phase.sh` — expected green, no production code changed this
  phase (see phase run output for actual result).
- Live: `bash scripts/dbx-verify.sh` green before W1 (after one token re-mint mid-session).
- Live: three `smelt run --target databricks` windows, each producing a report under
  `.smelt/targets/databricks/reports/`; `intervals.json` coverage ends at `2026-08-10` for all
  8 tracked models except `gold.events_enriched`.
