# Phase 9f plan — resume the live replay under 9e's ledgerless rebuild, and restore the report gates

## Objective

9e landed the missing half of the succession fix (`emit_succession_full_rebuild_ledgerless`, reached
by `rebuild_succession_state`'s `cell.state_downgraded` branch), proven against a real DuckDB
backend. The Databricks workspace is still staged exactly where 9d left it — 8 fixture days loaded
(`workspace.smelt_dogfood.github_events`, 26,220 rows), no model tables, oracle schema empty. This
phase resumes `phases/09d-plan.md` at its **task 4**: full refresh, windows 9/10/11 each with its
oracle refresh, both sweeps re-run, `08-parity.json` and `09b-equivalence.json` committed, and the
deferred parity gates restored. Closes criterion 7 and re-closes criterion 8 (rows 8, 9b and 9d come
off `## Blocked`).

**Live phase.** If `scripts/dbx-dogfood-env.sh` + `scripts/dbx-auth.sh` cannot reach the workspace,
stop and emit `<<PHASE_BLOCKED>>` — never skip green.

## Spec delta

None. 9d landed the `docs/specs/state.md` §"The degradation contract" paragraph on the
succession-grain recompute region, and 9e sharpened it with what the downgraded rebuild writes and
skips. The Databricks user-facing page is phase 10's.

## Tests

- `the_committed_parity_report_shows_no_unregistered_difference` (offline, **new** — 9d wrote then
  reverted it) — every difference in the freshly committed `08-parity.json` is registered in
  `DBX_DIVERGENCE_REGISTRY`; twin of the oracle suite's
  `the_committed_equivalence_report_shows_no_violation`.
- `dbx_registry_entries_are_all_live` (offline, **restored**) — two-sided: every Databricks-half
  registry entry is witnessed by a difference in `08-parity.json`, and every difference in that
  report is registered. Mirrors `registry_entries_are_all_live`'s DuckDB-leg shape.
- `the_committed_equivalence_report_shows_no_violation` (offline, already a hard gate) — must stay
  green against the **refreshed** `09b-equivalence.json` with no new registry entry beyond
  `gold_events_enriched`'s registered `UnorderedColumnDivergence`; needing another is a finding to
  record, not a bound to invent.
- `the_equivalence_report_covers_every_model_at_every_checkpoint`,
  `the_committed_report_proves_the_oracle_read_the_shared_source`,
  `the_final_window_compares_every_relation_with_nothing_exempt` (offline, already present) — must
  stay green against the refreshed report.
- `databricks_incremental_matches_its_oracle_at_every_window` (live) and
  `duckdb_and_databricks_agree_on_every_model` (live) — the two sweeps, re-run under the fix.

## Tasks

1. Build the binary from HEAD (`cargo build -p smelt-cli`) so the live legs run under 9e's fix, not
   a stale artifact. Then `source scripts/dbx-dogfood-env.sh`; `bash scripts/dbx-auth.sh`. If either
   fails to reach the workspace, stop and block.
2. Confirm the staged state before touching it: `workspace.smelt_dogfood.github_events` at 26,220
   rows over days 2026-08-05..2026-08-12, no model tables, `workspace.smelt_dogfood_oracle` empty.
   A mismatch means 9d's staging drifted — re-run `dbx-dogfood-oracle.sh reset` plus the day loop
   from 9d task 4 rather than proceeding on unknown state. **Do not double-load** (9d's own bug):
   reset first if any reload is needed.
3. Full refresh: `smelt run --target databricks --full-refresh --allow-full-refresh
   --event-time-start 2026-08-05 --event-time-end 2026-08-13`. Expect a clean 16/16 with both
   succession models now completing via the ledgerless rebuild. Sanity gate before continuing: the
   16 model row counts must match phase 6/7b's recorded 8-day counts; a mismatch is reported, not
   papered over. If the refusal recurs, capture the error and statement and block.
4. For n in 9, 10, 11: `dbx-dogfood-oracle.sh window n`, `oracle n`, `snapshot n`. Refresh the token
   between checkpoints if the hour lapses.
5. Equivalence leg: `dbx-dogfood-oracle.sh manifest`, the live sweep with
   `SMELT_DBX_DOGFOOD_LIVE=1 EQUIVALENCE_REPORT_OUT=…/phases/09b-equivalence.json`, then `report`.
   Commit the refreshed JSON and its `09b-equivalence.md` twin. `silver_actor_naming` is expected to
   match exactly at all three checkpoints.
6. Parity leg: `dbx-dogfood-parity.sh duck` (11 days), `dbx-snapshot`, `manifest`, the live sweep
   writing `phases/08-parity.json`, then `report`. `silver_actor_naming`'s `databricks_only=520` must
   now be `0`; if it is not, the fix is incomplete — record the measurement and block rather than
   registering a bound.
7. Land the two parity gates (task list's first two tests) against the now-existing `08-parity.json`,
   red-green, and delete the deferral comments at `github_activity_dual_target.rs:34`, `:1118-1135`
   and `:1260-1263`.
8. Flip rows 8, 9b and 9d to `done`; mark all three `## Blocked` entries RESOLVED by 9f (keeping
   their text, as the 9c entry does); append the dated Decision log entry with `silver_actor_naming`'s
   measured before/after (520 → 0) and the clean-16/16 refresh.
9. Write `phases/09f-summary.md`: the measured parity and equivalence tables, the sequence actually
   run, any Free-Edition quota or cold-start cost worth adding to `free-edition-facts.md`, and what
   phase 10's findings handoff should carry from here (including the 9c→9e two-step lesson about
   dispatch-only offline verification).

## Verification

- `bash .claude/scripts/verify-phase.sh` — the standing gate.
- `cargo test -p smelt-cli --test github_activity_dual_target` — offline legs, with both restored
  report gates green against the fresh `08-parity.json`.
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — offline legs green against the
  refreshed `09b-equivalence.json`.
- The two live sweeps above, each with `SMELT_DBX_DOGFOOD_LIVE=1`.
- `cargo test -p smelt-runtime --test succession_downgraded_rebuild --test statement_parity` and
  `cargo test -p smelt-cli --test maintenance_conformance` — untouched by this phase; run to confirm.

## Commit message

`outcome(databricks-dogfood-spine): phase 9f closes criteria 7 and 8 on post-fix live numbers`
