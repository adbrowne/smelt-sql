# Phase 11c plan — deploy the bundle and prove three scheduled runs

## Objective

Close criterion 11: `databricks bundle deploy` to the dogfood target, the Volume seeded, the
schedule enabled, **three consecutive scheduled runs** completing without a manual trigger, their
run reports pulled from the Volume, the resulting state compared against a full-refresh oracle
exactly as criterion 8 checks, the Volume FUSE layer's `.smelt/lock` and rename-atomicity
behaviour measured, and the compute consumed recorded against criterion 4's Free Edition quotas.
Completing this row completes the outcome.

**Live row.** If `scripts/dbx-dogfood-env.sh` cannot reach the workspace (`SMELT_DBX_HOST` unset
or `bash scripts/dbx-verify.sh` red), do tasks 1–2 offline if they are still pending, then emit
`<<PHASE_BLOCKED>>` — never skip a live leg green.

## Cadence compression (how three *scheduled* runs fit in one sitting)

A daily cron needs three calendar days, which no headless step can wait out. The committed
default stays daily; the cron expression becomes a bundle **variable** so the proof deploy can
pass `--var schedule_cron='0 0/20 * * * ?'` (every 20 minutes), let three runs fire ~40 minutes
apart, then redeploy at the committed default. The runs are genuinely schedule-triggered
(`trigger: PERIODIC`), which is the criterion's substance; cadence is a test parameter.

## Spec delta

- `docs-site/docs/guide/targets.md` §"Deployment: Databricks Asset Bundle" — document the
  `schedule_cron` variable and the `volume_probe` job, and add the measured Volume-FUSE facts.
- **Conditional:** if the probe shows the Volume's FUSE layer does *not* honour advisory locks
  (`flock`) or rename-atomicity, add a Known Divergence to `docs/specs/run_state.md` stating what
  `.smelt/` guarantees do and do not hold on a Unity Catalog Volume. If it does honour them,
  record the positive result in `targets.md` only — no spec change.

## Tests

Offline (per-PR, no workspace):
1. `databricks_bundle.rs::bundle_declares_one_daily_scheduled_serverless_job` — **amended**: the
   schedule reads `${var.schedule_cron}` and that variable's committed default is the daily
   expression, so the per-PR artifact is still a daily job.
2. `databricks_bundle.rs::bundle_schedule_is_explicitly_unpaused` — the schedule declares
   `pause_status: UNPAUSED`, so a deploy enables it rather than relying on an API default.
3. `databricks_bundle.rs::bundle_volume_probe_targets_the_declared_volume` — the probe job's
   Volume path composes from the same `${var.catalog}/${var.schema}/${var.volume_name}` refs the
   `smelt_run` task uses, so probe and job cannot measure different paths.
4. `databricks_bundle.rs::databricks_bundle_validate_is_clean` — unchanged, must stay green with
   the new variable and resource present.

Report-driven gates (red until this phase's evidence lands, then hard):
5. `github_activity_dbx_scheduled.rs::three_consecutive_scheduled_runs_succeeded` — reads the
   committed `phases/11c-runs.json`: exactly three records, each `trigger` PERIODIC (never
   `RUN_JOB_TASK`/manual), each terminal state SUCCESS, each with both tasks, in ascending
   start-time order with no intervening manual run.
6. `github_activity_dbx_scheduled.rs::scheduled_runs_advanced_the_fixture` — each run's loader
   task landed a *distinct*, strictly increasing fixture day, proving `--next-day` self-drives
   and runs 2 and 3 are genuine incremental windows over run 1.
7. `github_activity_dbx_scheduled.rs::scheduled_state_matches_its_full_refresh_oracle` — reads
   the committed `phases/11c-equivalence.json`: every relation equal, or a difference that
   matches the existing `EQUIVALENCE_DIVERGENCE_REGISTRY` entry
   (`gold_events_enriched`'s `UnorderedColumnDivergence`); an unregistered one fails.

## Tasks

1. Add the `schedule_cron` variable (default `0 0 6 * * ?`) and explicit `pause_status: UNPAUSED`
   to `databricks.yml`/`resources/github_activity_job.yml`; drive tests 1–2 red-green.
2. Add `examples/github_activity/dbx_job/volume_probe.py` and a `volume_probe` job resource that
   runs it on serverless against the declared Volume path: attempt an `flock` on
   `<project>/.smelt/lock`, a `os.replace()` rename over an existing file, an `fsync`, and a
   concurrent-writer second acquisition; print a JSON verdict per probe. Drive test 3 red-green.
3. Rebuild `.smelt-dbx-venv` via `bash scripts/dbx-dogfood-venv.sh` so the new `duckdb` pin lands
   and 11b's module/CLI Arrow-byte parity test runs instead of skipping (11b summary's carry).
4. `source scripts/dbx-dogfood-env.sh && bash scripts/dbx-auth.sh && bash scripts/dbx-verify.sh`
   — if red, block per the live-row rule above.
5. `bash scripts/dbx-bundle.sh deploy` (Volume resource created), **then**
   `bash scripts/dbx-bundle.sh seed`, then confirm `smelt.yml` and `models/` are on the Volume
   and `.smelt/` is untouched. Order matters — the Volume must exist before `seed` writes.
6. `bash scripts/dbx-bundle.sh run volume_probe`, capture its JSON verdict into
   `phases/11c-volume-probe.md`, and apply the conditional spec delta from that result.
7. Redeploy with `--var schedule_cron='0 0/20 * * * ?'`, confirm the schedule is UNPAUSED, and
   wait for three consecutive scheduled runs. Poll with `databricks jobs list-runs` through the
   wrapper in the **foreground** (a bounded `until` loop; never `run_in_background`). Trigger
   nothing manually.
8. Capture the three run records (run id, trigger type, start/end, per-task state, duration,
   loader day) into `phases/11c-runs.json`; drive tests 5–6 red-green against it.
9. Pull the Volume's run state — `.smelt/` run reports and manifests — down to
   `target/phase11c/` and summarise which models each run touched in `phases/11c-runs.md`.
10. Run the criterion-8 comparison on the post-run-3 state: `bash scripts/dbx-dogfood-oracle.sh
    oracle <n>` + `snapshot <n>` + `manifest`, then the equivalence test writing
    `phases/11c-equivalence.json`; drive test 7 red-green.
11. Redeploy at the committed daily default and confirm the deployed schedule reads
    `0 0 6 * * ?` and is UNPAUSED — the workspace is left in the shape the committed bundle
    describes.
12. Record the compute the schedule consumed (per-run serverless duration, total across the
    proof window, cold-start latency) into
    `docs/outcomes/20260912-databricks-dogfood-spine/free-edition-facts.md` (criterion 4's
    facts sheet) and append the scheduled-run findings to
    `docs/handoffs/2026-09-13-databricks-findings.md`.
13. Update `docs-site/docs/guide/targets.md` per the spec delta.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test databricks_bundle --test dbx_dogfood_loader`
- `bash scripts/dbx-bundle.sh validate`
- `SMELT_DBX_DOGFOOD_LIVE=1 cargo test -p smelt-cli --test github_activity_dbx_scheduled`
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` (must stay green)

## Commit message

`outcome(databricks-dogfood-spine): phase 11c deploys the bundle and proves three scheduled runs against the oracle`
