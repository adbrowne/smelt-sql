# Phase 11k — Live: seed the job's frontier and close criterion 11

## Objective

Resume 11i from its remaining live legs, now unblocked by 11j's bootstrap tool: seed
`databricks_job`'s Volume-resident interval history from the fixture days already ingested
into `workspace.smelt_dogfood`, then get the scheduled job past `--auto`'s refusal and through
**three consecutive scheduled runs**, exactly as criterion 11 and 11i's five prior fixes were
building toward. Everything 11i already fixed (dual-wheel artifact split, `smelt_env`
dependency resolution, wildcard expansion, `${workspace.file_path}` sync-root nesting, the
wheel's `LD_LIBRARY_PATH`, and the two inert placeholder env vars) is committed and does not
need to be redone — this phase only adds the seed step ahead of the smoke run.

## Prerequisites

- `bash scripts/dbx-verify.sh` green (reachability + out-of-scope-write refusal) before
  starting — re-run it fresh even if a recent pass already confirmed it, since the credential
  has expired mid-flight before (see outcome `## Blocked` history).
- 11j merged: `smelt state seed-interval` exists and its offline tests pass.

## Tasks

1. **Discover the real ingestion frontier**, not an assumed date: query
   `workspace.smelt_dogfood.github_events` for `max(ingested_date)` (and the count of distinct
   `ingested_date` values, to sanity-check against the outcome's "12+ days" running total).
   Record both in `phases/11k-summary.md`. Do not hardcode a date from this plan or from a
   prior phase's summary — the live schema is the source of truth.
2. **Seed the Volume's interval file, one model at a time, locally.** In a scratch project
   directory (a fresh checkout of `examples/github_activity/` is fine — the tool only reads
   compiled model SQL and writes `.smelt/`), run `smelt state seed-interval --target
   databricks_job --model <m> --start <fixture-day-1> --end <task-1-max-date>` for every model
   in the pipeline's execution order (all 16 — get the list from `smelt explain` or the
   dependency graph, don't hand-type it). `--start` is the fixture's first loaded day (also
   confirmed by task 1's query, `min(ingested_date)`).
3. **Upload exactly one file to the Volume**: `databricks fs cp --overwrite
   <scratch>/.smelt/targets/databricks_job/intervals.json
   dbfs:/Volumes/<catalog>/<schema>/<volume>/project/.smelt/targets/databricks_job/intervals.json`.
   Do **not** touch `.smelt/lock`, `meta.json`, or any other target's subtree — this is a
   single-file `fs cp`, not a directory sync, so there is no risk of clobbering state a smoke
   run may already have written under a concurrent path. Confirm via `databricks fs cat` (or
   a second `fs cp` back down) that the uploaded content matches what was generated locally.
4. **Redeploy** (`scripts/dbx-bundle.sh deploy`) to pick up 11i's fixes if the bundle is not
   already at that state; re-seed `smelt.yml`/`models/` (`scripts/dbx-bundle.sh seed`) — this
   step never touches `.smelt/`, so it is safe to run after task 3.
5. **One manual smoke run** (`scripts/dbx-bundle.sh run`), not yet on the compressed cadence.
   Confirm from the run's own logs (or a `runs get` call) that `smelt run --auto` picked a
   window starting at the day after task 1's max date, rather than refusing. If it still
   refuses, the seed did not reach the path the job actually reads — check for a
   `${workspace.file_path}` vs `dbfs:/Volumes/...` path mismatch (11i task 4's lesson) before
   assuming the seed tool itself is wrong.
6. **Compress the cadence** (`--var schedule_cron=...`, same mechanism 11c/11e/11g/11i already
   used) and let **three consecutive scheduled runs** (`trigger: PERIODIC`, never manual)
   complete.
7. **Pull run reports from the Volume** for all three runs; compare the resulting state
   against a full-refresh oracle exactly as criterion 8's comparator does, accounting for
   every fixture day now loaded (task 1's count plus the loader's `--next-day` advances from
   the three scheduled runs).
8. **Record compute consumed** against the Free Edition quotas of criterion 4.
9. **Write up the `volume_probe` verdict** (already committed in 11c) in `docs-site/`, if not
   already done by a prior phase's summary.
10. **Restore the committed daily cadence** (`0 0 6 * * ?`).
11. Write `phases/11k-summary.md` with all of the above evidence. If any step blocks on
    something 11j did not anticipate (e.g. the seed reaching the wrong path, or a genuinely new
    infra defect), record it in `## Blocked` the same way 11i's five bugs were recorded — fix
    what's fixable in this pass, leave a design question for the next planner if one appears.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- The three `github_activity_dbx_scheduled.rs` gates (task 1 of 11c's plan) flip from
  skip-when-missing to hard, since this phase lands their evidence — matching 11e/11g/11i's
  precedent.
- No ratchet lowered.

## Commit message

`feat(databricks): seed the job's frontier from ingested history; close criterion 11`

## If this closes criterion 11

Update `outcome.md`: flip row 11k to `done`, close criterion 11 in the outcome header/body,
move the outcome `Status` to `done`, and remove the now-resolved parts of `## Blocked` (leaving
the general `--auto`/backend-resident-state question as a pointer to
`docs/specs/run_state.md` §"Known Divergences / Open Questions" rather than deleting it — that
question is still open for other cloud targets).
