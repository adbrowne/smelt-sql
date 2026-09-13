# Phase 11c plan (revised 2026-09-13) — resume the deploy and prove three scheduled runs

**Resumes the blocked first attempt.** Tasks 1–4 of the original 11c plan (git:
`4d6952a2a^:docs/outcomes/.../phases/11c-plan.md`) are committed and green — the
`schedule_cron` bundle variable, `pause_status: UNPAUSED`, the `volume_probe` job and script,
the `dbx-bundle.sh` token-export fix, and the rebuilt `.smelt-dbx-venv`. The `CREATE VOLUME`
grant that blocked `deploy` was applied by the owning identity (`scripts/dbx-grant-volume.sql`,
decision log 2026-09-13). This plan renumbers from the deploy onwards and adds the run-polling
wrapper the original plan assumed existed.

## Objective

Close criterion 11: deploy the bundle to the dogfood target, create and seed the Volume, measure
the Volume FUSE layer with the committed `volume_probe` job, let **three consecutive scheduled**
(`trigger: PERIODIC`) runs complete under a `--var`-compressed cadence, pull their run reports,
compare the resulting state against a full-refresh oracle exactly as criterion 8 does, restore
the committed daily cadence, and record the compute consumed against criterion 4's Free Edition
quotas. This is the outcome's last row.

**Live row.** If `scripts/dbx-verify.sh` is red (or `SMELT_DBX_HOST` unset), land tasks 1–2
offline and emit `<<PHASE_BLOCKED>>` — never skip a live leg green. Likewise if `deploy` still
refuses to create the managed Volume under the schema-level grant: block, naming
`databricks bundle deployment bind` against a human-pre-created Volume as the candidate route
(carried forward from the 11c summary's open question).

## Spec delta

- `docs-site/docs/guide/targets.md` §"Deployment: Databricks Asset Bundle" — document the
  `schedule_cron` variable, the `volume_probe` job, and the measured Volume-FUSE facts.
- **Conditional:** if the probe shows the Volume's FUSE layer does not honour `flock` advisory
  locking or `os.replace()` atomicity, add a Known Divergence to `docs/specs/run_state.md`
  stating which `.smelt/` guarantees do and do not hold on a Unity Catalog Volume. If it does
  honour them, record the positive result in `targets.md` only.

## Tests

Offline (per-PR, no workspace):
1. `databricks_bundle.rs::every_live_subcommand_exports_both_credentials` — structural over
   `scripts/dbx-bundle.sh`: each branch that invokes `databricks` against a real workspace
   exports `DATABRICKS_HOST` *and* `DATABRICKS_TOKEN`. Locks in the 11c token bug fix, which
   would otherwise silently break every future `deploy`/`run`/`seed`/`runs`.
2. `databricks_bundle.rs::runs_subcommand_is_read_only` — the new `runs` branch invokes only
   `jobs list-runs` / `jobs get-run`; no mutating `databricks` verb appears in it.
3. The four tests already green (`bundle_declares_one_daily_scheduled_serverless_job`,
   `bundle_schedule_is_explicitly_unpaused`, `bundle_volume_probe_targets_the_declared_volume`,
   `databricks_bundle_validate_is_clean`) must stay green — no regression.

Report-driven gates (red until this phase's evidence lands, then hard):
4. `github_activity_dbx_scheduled.rs::three_consecutive_scheduled_runs_succeeded` — reads the
   committed `phases/11c-runs.json`: exactly three records, each `trigger` PERIODIC (never
   manual/`RUN_JOB_TASK`), each terminal state SUCCESS with both tasks, ascending start time,
   no intervening manual run.
5. `github_activity_dbx_scheduled.rs::scheduled_runs_advanced_the_fixture` — each run's loader
   task landed a distinct, strictly increasing fixture day, proving `--next-day` self-drives and
   that runs 2 and 3 are genuine incremental windows over run 1's refresh.
6. `github_activity_dbx_scheduled.rs::scheduled_state_matches_its_full_refresh_oracle` — reads
   the committed `phases/11c-equivalence.json`: every relation equal, or a difference matching
   the existing `EQUIVALENCE_DIVERGENCE_REGISTRY` entry (`gold_events_enriched`'s
   `UnorderedColumnDivergence`); an unregistered difference fails.

## Tasks

1. Add a read-only `runs` subcommand to `scripts/dbx-bundle.sh` (`runs list <job-name>` →
   resolve the job id then `databricks jobs list-runs --job-id`; `runs get <run-id>` →
   `jobs get-run`), exporting both credentials; drive tests 1–2 red-green. No
   `.claude/settings.json` change needed — `dbx-bundle.sh*` is already allow-listed.
2. `source scripts/dbx-dogfood-env.sh && bash scripts/dbx-auth.sh && bash scripts/dbx-verify.sh`
   — if red, block per the live-row rule.
3. `bash scripts/dbx-bundle.sh deploy` — the Volume resource must now create. If it still
   refuses, block per the live-row rule.
4. `bash scripts/dbx-bundle.sh seed`, then confirm `smelt.yml` and `models/` are on the Volume
   and `.smelt/` is untouched. Order matters: the Volume must exist before `seed` writes.
5. `bash scripts/dbx-bundle.sh run github_activity_volume_probe`; capture its JSON verdict into
   `phases/11c-volume-probe.md` and apply the conditional spec delta from that result.
6. Redeploy with `--var schedule_cron='0 0/20 * * * ?'`; confirm the deployed schedule is
   UNPAUSED and reads the compressed expression.
7. Wait for three consecutive scheduled runs, polling with `dbx-bundle.sh runs list` in a
   bounded **foreground** `until` loop (never `run_in_background`). Trigger nothing manually.
   Budget one `dbx-auth.sh` remint during the ~40-minute wait if the token TTL bites (the gpg
   cache TTL is now 43200s, so this should not recur).
8. Capture the three run records (run id, trigger type, start/end, per-task state, duration,
   loader day) into `phases/11c-runs.json`; drive tests 4–5 red-green against it.
9. Pull the Volume's `.smelt/` run reports and manifests to `target/phase11c/` and summarise
   which models each run touched in `phases/11c-runs.md`.
10. Run the criterion-8 comparison on the post-run-3 state: `bash scripts/dbx-dogfood-oracle.sh
    oracle <n>` + `snapshot <n>` + `manifest`, then the equivalence sweep writing
    `phases/11c-equivalence.json`; drive test 6 red-green.
11. Redeploy at the committed daily default; confirm the deployed schedule reads `0 0 6 * * ?`
    and is UNPAUSED — the workspace is left in the shape the committed bundle describes.
12. Record the compute the schedule consumed (per-run serverless duration, total across the
    proof window, cold-start latency) in `free-edition-facts.md`, and append the scheduled-run
    findings to `docs/handoffs/2026-09-13-databricks-findings.md`.
13. Update `docs-site/docs/guide/targets.md` per the spec delta.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test databricks_bundle --test dbx_dogfood_loader`
- `bash scripts/dbx-bundle.sh validate`
- `SMELT_DBX_DOGFOOD_LIVE=1 cargo test -p smelt-cli --test github_activity_dbx_scheduled`
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` (must stay green)

## Commit message

`outcome(databricks-dogfood-spine): phase 11c deploys the bundle and proves three scheduled runs against the oracle`
