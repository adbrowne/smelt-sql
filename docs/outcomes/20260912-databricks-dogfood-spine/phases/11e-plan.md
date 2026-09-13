# Phase 11e plan — resume 11c's live legs under 11d's ambient fix

**Resumes `phases/11c-plan.md` from its task 7.** 11c's tasks 1–6 are committed and green
(the `runs` subcommand and its two structural tests, the live `deploy`, the Volume creation and
`seed`, the `volume_probe` verdict in `phases/11c-volume-probe.md`, the compressed-cadence
redeploy) and stay done. 11d closed the ambient-host gap that blocked task 7. This plan
renumbers from a fresh deploy (11d changed the Rust/Python session path, the loader and
`smelt.yml`, so the deployed wheel and the seeded project are both stale) onwards.

## Objective

Close criterion 11, the outcome's last open criterion: three consecutive **scheduled**
(`trigger: PERIODIC`) runs of the deployed bundle job complete, their run reports are pulled
from the Volume, the state they leave is compared against a full-refresh oracle exactly as
criterion 8 checks, the compute they consumed is recorded against criterion 4's Free Edition
quotas, the `volume_probe` verdict is written up in `docs-site/`, and the committed daily
cadence is restored. Also closes 11c's unlanded task 13 (docs) and tasks 8–12 (evidence).

**Live row.** If `scripts/dbx-verify.sh` is red (or the credential cannot be reminted), land
task 1's test scaffold offline and emit `<<PHASE_BLOCKED>>` — never skip a live leg green.

## Spec delta

- `docs-site/docs/guide/targets.md` §"Deployment: Databricks Asset Bundle" — document the
  `schedule_cron` bundle variable and the measured Volume-FUSE facts (`flock` advisory locking,
  `os.replace()` rename atomicity and `fsync` all honoured; from `phases/11c-volume-probe.md`).
  The probe came back positive, so no Known Divergence goes into `docs/specs/run_state.md`
  (11c's conditional delta resolves to the positive branch).
- No other spec change: 11e proves already-specified behaviour, it does not change it.

## Tests

New file `crates/smelt-cli/tests/github_activity_dbx_scheduled.rs` (11c plan tests 4–6, never
written). Report-driven: red until this phase's evidence lands, then hard gates.

1. `three_consecutive_scheduled_runs_succeeded` — reads the committed `phases/11e-runs.json`:
   exactly three records, each `trigger` PERIODIC (never manual / `RUN_JOB_TASK`), each terminal
   state SUCCESS with both tasks succeeded, strictly ascending start time, and no manual run
   interleaved between them.
2. `scheduled_runs_advanced_the_fixture` — each run's loader task landed a distinct, strictly
   increasing fixture day, proving `--next-day` self-drives and that the runs are genuine
   successive windows rather than repeats.
3. `scheduled_state_matches_its_full_refresh_oracle` — reads the committed
   `phases/11e-equivalence.json`: every relation equal, or a difference matching the existing
   `EQUIVALENCE_DIVERGENCE_REGISTRY` entry in `github_activity_dbx_oracle.rs`
   (`gold_events_enriched`'s `UnorderedColumnDivergence`); an unregistered difference fails.
   Reuse that registry by importing the shared `github_activity_support` seam rather than
   restating it.

Must stay green, no regression: `databricks_bundle.rs` (16 tests incl. 11d's
`job_target_is_ambient`), `dbx_dogfood_loader.rs`, `github_activity_dbx_oracle.rs`.

## Tasks

1. Write the three tests above against the not-yet-existing report paths (red).
2. `source scripts/dbx-dogfood-env.sh && bash scripts/dbx-auth.sh && bash scripts/dbx-verify.sh`
   — if red, block per the live-row rule.
3. `bash scripts/dbx-bundle.sh deploy` (committed daily cadence) — rebuilds and uploads the
   wheel carrying 11d's ambient fix; then `bash scripts/dbx-bundle.sh seed` to refresh the
   Volume's `smelt.yml`/`models/` (11d dropped `host:` from the `databricks_job` target).
   Confirm `.smelt/` on the Volume is untouched by `seed`.
4. **Smoke first, before the cadence is compressed:** `bash scripts/dbx-bundle.sh run
   github_activity_daily` once, manually, and confirm both tasks succeed end to end under the
   ambient session. Fix anything it surfaces here rather than burning a 20-minute cron cycle per
   discovery. This manual run is deliberately *before* the three scheduled ones (test 1 forbids
   only an interleaved manual run) and leaves `.smelt/` holding a first full refresh, so all
   three scheduled runs are genuine incremental windows.
5. Redeploy with `--var schedule_cron='0 0/20 * * * ?'`; confirm the deployed schedule reads the
   compressed expression and is UNPAUSED.
6. Wait for three consecutive PERIODIC runs to complete, polling `bash scripts/dbx-bundle.sh
   runs list github_activity_daily` in a bounded **foreground** `until` loop (never
   `run_in_background`). Trigger nothing manually during the wait; remint the token with
   `dbx-auth.sh` if the ~45-minute TTL bites.
7. Capture the three run records (run id, trigger type, start/end, per-task state, duration,
   loader fixture day) into `phases/11e-runs.json`; drive tests 1–2 green.
8. Pull the Volume's `.smelt/` run reports and manifests to `target/phase11e/` and summarise
   which models each run touched in `phases/11e-runs.md`.
9. Criterion-8 comparison on the post-run-3 state: `bash scripts/dbx-dogfood-oracle.sh oracle
   <n>`, `snapshot <n>`, `manifest`, then the equivalence sweep writing
   `phases/11e-equivalence.json`; drive test 3 green.
10. `bash scripts/dbx-bundle.sh deploy` at the committed default; confirm the deployed schedule
    reads `0 0 6 * * ?` and is UNPAUSED — the workspace matches the committed bundle again.
11. Record the compute consumed (per-run serverless duration, cold-start latency, total across
    the proof window) in `free-edition-facts.md`, and append the scheduled-run findings to
    `docs/handoffs/2026-09-13-databricks-findings.md`.
12. Land the docs-site spec delta above.
13. Write `phases/11e-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_dbx_scheduled --test databricks_bundle --test dbx_dogfood_loader`
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` (must stay green)
- `bash scripts/dbx-bundle.sh validate`

## Commit message

`outcome(databricks-dogfood-spine): phase 11e proves three scheduled runs against the oracle`
