# Phase 11i plan — resume 11g's live legs under 11h's dual-arch wheel

**Resumes `phases/11g-plan.md` from its task 3.** 11g's tasks 1–2 are landed (the three gates in
`github_activity_dbx_scheduled.rs` already point at `phases/11g-runs.json` /
`phases/11g-equivalence.json`; the cp311 pin is committed). 11h closed the last wheel defect:
`scripts/dbx-wheel-build.sh` now emits both a `manylinux_2_28_x86_64` and a
`manylinux_2_28_aarch64` `cp311` wheel, and `github_activity_job.yml` selects between them with
`platform_machine` markers. This plan redeploys under those wheels and drives the live legs to
the end.

**Live row.** `bash scripts/dbx-verify.sh` was run green during this planning pass (both dogfood
schemas reachable, out-of-scope write refused). If it is red at implement time and
`scripts/dbx-auth.sh` cannot remint, emit `<<PHASE_BLOCKED>>` — never skip a live leg green.

## Objective

Close criterion 11, the outcome's last open criterion: the deployed bundle installs smelt on
serverless compute on **either** architecture, three consecutive **scheduled** (`trigger:
PERIODIC`) runs complete, their run reports are pulled from the Volume, the state they leave
equals a full-refresh oracle exactly as criterion 8 checks, the compute consumed is recorded
against criterion 4's Free Edition quotas, the `volume_probe` verdict is written up in
`docs-site/`, and the committed daily cadence is restored.

## Spec delta

- `docs-site/docs/guide/targets.md` §"Deployment: Databricks Asset Bundle" — document the
  `schedule_cron` bundle variable and the measured Volume-FUSE facts (`flock` advisory locking,
  `os.replace()` rename atomicity and `fsync` all honoured; source `phases/11c-volume-probe.md`).
  The probe was positive, so no Known Divergence goes into `docs/specs/run_state.md`.
- No other spec change: this phase proves already-specified behaviour.

## Tests

The three gates exist and currently **skip** when their evidence files are absent. This phase
lands the evidence and flips all three to hard gates (the 9b precedent). Evidence paths stay
`11g-*` — the tests already point there, and renaming per resume attempt costs a red/green cycle
with no information gain.

1. `three_consecutive_scheduled_runs_succeeded` — `phases/11g-runs.json`: exactly three records,
   each `trigger` PERIODIC (never manual / `RUN_JOB_TASK`), each terminal state SUCCESS with both
   tasks succeeded, strictly ascending start time, no manual run interleaved.
2. `scheduled_runs_advanced_the_fixture` — each run's loader task landed a distinct, strictly
   increasing fixture day. The workspace holds 12 days (`2026-08-05`..`2026-08-16`) plus whatever
   task 4's smoke run adds; assert ascending distinctness, never hardcoded dates.
3. `scheduled_state_matches_its_full_refresh_oracle` — `phases/11g-equivalence.json`: every
   relation equal, or a difference matching `DATABRICKS_EQUIVALENCE_DIVERGENCE_REGISTRY` in
   `parity_support/mod.rs`; an unregistered difference fails.

Must stay green: `databricks_bundle.rs`, `dbx_dogfood_loader.rs`, `github_activity_dbx_oracle.rs`.

## Tasks

1. Flip the three gates from skip-on-missing to hard failure (red step): absent evidence is a
   test failure, not an `eprintln!` skip. Update the module doc comment accordingly.
2. `source scripts/dbx-dogfood-env.sh && bash scripts/dbx-auth.sh && bash scripts/dbx-verify.sh` —
   if red, block per the live-row rule.
3. `bash scripts/dbx-wheel-build.sh` (both arches) then `bash scripts/dbx-bundle.sh deploy` at the
   committed daily cadence; confirm **both** uploaded wheel filenames carry `cp311` and
   `manylinux_2_28`, one `_x86_64` and one `_aarch64`. If the wheel build trips over the
   root-owned `target/release/build/ZZZ-orphan-*` / `target/maturin-root-owned-orphan-*` dirs 11h
   left behind, rename aside within the same filesystem again rather than blocking.
4. `bash scripts/dbx-bundle.sh seed`; confirm `.smelt/` on the Volume is untouched by `seed`.
5. **Smoke first, before the cadence is compressed:** `bash scripts/dbx-bundle.sh run
   github_activity_daily` once, manually. Confirm `smelt_run` installs smelt on whichever arch it
   lands on and both tasks succeed end to end. Fix what it surfaces here rather than burning a
   cron cycle per discovery. This manual run precedes the three scheduled ones and leaves
   `.smelt/` holding a completed window, so all three scheduled runs are genuine incrementals.
6. Redeploy with `--var schedule_cron='0 0/20 * * * ?'`; confirm the deployed schedule reads the
   compressed expression and is UNPAUSED.
7. Wait for three consecutive PERIODIC runs, polling `bash scripts/dbx-bundle.sh runs list
   github_activity_daily` in a bounded **foreground** `until` loop (never `run_in_background`).
   Trigger nothing manually during the wait; remint with `dbx-auth.sh` if the ~45-minute TTL bites.
   If a scheduled run lands on `aarch64` and still fails to install, capture the driver log and
   block with that evidence rather than retrying blind.
8. Capture the three run records (run id, trigger type, start/end, per-task state, duration,
   loader fixture day) into `phases/11g-runs.json`; drive tests 1–2 green.
9. Pull the Volume's `.smelt/` run reports and manifests to `target/phase11i/` and summarise which
   models each run touched in `phases/11i-runs.md`.
10. Criterion-8 comparison on the post-run-3 state: `bash scripts/dbx-dogfood-oracle.sh oracle <n>`,
    `snapshot <n>`, `manifest`, then the equivalence sweep writing `phases/11g-equivalence.json`;
    drive test 3 green.
11. `bash scripts/dbx-bundle.sh deploy` at the committed default; confirm the deployed schedule
    reads `0 0 6 * * ?` and is UNPAUSED — the workspace matches the committed bundle again.
12. Record the compute consumed (per-run serverless duration, cold-start latency, total across the
    proof window, and which arch each run landed on) in `free-edition-facts.md`, and append the
    scheduled-run findings — including the dual-arch root cause and its fix — to
    `docs/handoffs/2026-09-13-databricks-findings.md`.
13. Land the docs-site spec delta above.
14. Write `phases/11i-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_dbx_scheduled --test databricks_bundle --test dbx_dogfood_loader`
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` (must stay green)
- `bash scripts/dbx-bundle.sh validate`

## Commit message

`outcome(databricks-dogfood-spine): phase 11i proves three scheduled runs against the oracle`
