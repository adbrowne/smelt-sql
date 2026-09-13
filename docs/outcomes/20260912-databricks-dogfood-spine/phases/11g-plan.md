# Phase 11g plan — resume 11e's live legs under 11f's manylinux wheel

**Resumes `phases/11e-plan.md` from its task 3.** 11e's tasks 1–2 are committed and green (the
`github_activity_dbx_scheduled.rs` scaffold, `dbx-verify.sh`), and its four ambient-session fixes
plus the loader's `RecordBatchReader` fix are landed. 11f closed the fifth defect: the bundle's
`smelt_wheel` artifact now builds through `scripts/dbx-wheel-build.sh` at a `manylinux_2_28`
floor. This plan redeploys from scratch under that wheel and drives the live legs to the end.

## Objective

Close criterion 11, the outcome's last open criterion: the deployed bundle installs smelt on
serverless compute, three consecutive **scheduled** (`trigger: PERIODIC`) runs complete, their
run reports are pulled from the Volume, the state they leave is compared against a full-refresh
oracle exactly as criterion 8 checks, the compute consumed is recorded against criterion 4's
Free Edition quotas, the `volume_probe` verdict is written up in `docs-site/`, and the committed
daily cadence is restored.

**Live row.** If `scripts/dbx-verify.sh` is red (or the credential cannot be reminted), emit
`<<PHASE_BLOCKED>>` — never skip a live leg green.

## Spec delta

- `docs-site/docs/guide/targets.md` §"Deployment: Databricks Asset Bundle" — document the
  `schedule_cron` bundle variable and the measured Volume-FUSE facts (`flock` advisory locking,
  `os.replace()` rename atomicity and `fsync` all honoured; source `phases/11c-volume-probe.md`).
  The probe was positive, so no Known Divergence goes into `docs/specs/run_state.md`.
- No other spec change: this phase proves already-specified behaviour.

## Tests

The three gates already exist in `crates/smelt-cli/tests/github_activity_dbx_scheduled.rs` and
currently **skip** because their evidence files are absent. This phase lands the evidence and
flips all three from skip-when-missing to hard gates (the 9b precedent):

1. `three_consecutive_scheduled_runs_succeeded` — `phases/11g-runs.json`: exactly three records,
   each `trigger` PERIODIC (never manual / `RUN_JOB_TASK`), each terminal state SUCCESS with both
   tasks succeeded, strictly ascending start time, no manual run interleaved.
2. `scheduled_runs_advanced_the_fixture` — each run's loader task landed a distinct, strictly
   increasing fixture day (the workspace already holds 12 days, `2026-08-05`..`2026-08-16`, so
   expect `2026-08-17` onward — assert ascending distinctness, not hardcoded dates).
3. `scheduled_state_matches_its_full_refresh_oracle` — `phases/11g-equivalence.json`: every
   relation equal, or a difference matching `DATABRICKS_EQUIVALENCE_DIVERGENCE_REGISTRY` in
   `parity_support/mod.rs`; an unregistered difference fails.

Point the three tests at the `11g-*` evidence paths (rename from `11e-*`) as the first, red step.
Must stay green: `databricks_bundle.rs`, `dbx_dogfood_loader.rs`, `github_activity_dbx_oracle.rs`.

## Tasks

1. Repoint the three tests at `phases/11g-runs.json` / `phases/11g-equivalence.json` and make
   absence a hard failure rather than a skip (red).
2. `source scripts/dbx-dogfood-env.sh && bash scripts/dbx-auth.sh && bash scripts/dbx-verify.sh`
   — if red, block per the live-row rule.
3. `bash scripts/dbx-bundle.sh deploy` (committed daily cadence) — rebuilds the wheel through
   `scripts/dbx-wheel-build.sh` and uploads it; confirm the uploaded wheel filename carries
   `manylinux_2_28`. Then `bash scripts/dbx-bundle.sh seed`; confirm `.smelt/` on the Volume is
   untouched by `seed`.
4. **Smoke first, before the cadence is compressed:** `bash scripts/dbx-bundle.sh run
   github_activity_daily` once, manually. Confirm `smelt_run` now *installs* smelt (the 11e
   blocker) and both tasks succeed end to end. Fix what it surfaces here rather than burning a
   cron cycle per discovery. This manual run precedes the three scheduled ones and leaves
   `.smelt/` holding a full refresh, so all three scheduled runs are genuine incremental windows.
5. Redeploy with `--var schedule_cron='0 0/20 * * * ?'`; confirm the deployed schedule reads the
   compressed expression and is UNPAUSED.
6. Wait for three consecutive PERIODIC runs, polling `bash scripts/dbx-bundle.sh runs list
   github_activity_daily` in a bounded **foreground** `until` loop (never `run_in_background`).
   Trigger nothing manually during the wait; remint with `dbx-auth.sh` if the ~45-minute TTL bites.
7. Capture the three run records (run id, trigger type, start/end, per-task state, duration,
   loader fixture day) into `phases/11g-runs.json`; drive tests 1–2 green.
8. Pull the Volume's `.smelt/` run reports and manifests to `target/phase11g/` and summarise which
   models each run touched in `phases/11g-runs.md`.
9. Criterion-8 comparison on the post-run-3 state: `bash scripts/dbx-dogfood-oracle.sh oracle <n>`,
   `snapshot <n>`, `manifest`, then the equivalence sweep writing `phases/11g-equivalence.json`;
   drive test 3 green.
10. `bash scripts/dbx-bundle.sh deploy` at the committed default; confirm the deployed schedule
    reads `0 0 6 * * ?` and is UNPAUSED — the workspace matches the committed bundle again.
11. Record the compute consumed (per-run serverless duration, cold-start latency, total across the
    proof window) in `free-edition-facts.md`, and append the scheduled-run findings — including
    the wheel-platform root cause and its fix — to `docs/handoffs/2026-09-13-databricks-findings.md`.
12. Land the docs-site spec delta above.
13. Write `phases/11g-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_dbx_scheduled --test databricks_bundle --test dbx_dogfood_loader`
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` (must stay green)
- `bash scripts/dbx-bundle.sh validate`

## Commit message

`outcome(databricks-dogfood-spine): phase 11g proves three scheduled runs against the oracle`
