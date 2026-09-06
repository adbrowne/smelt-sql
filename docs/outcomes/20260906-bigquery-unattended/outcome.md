# Outcome: The pipeline runs unattended on GCP, and a failure reaches me

**Created:** 2026-09-06
**Status:** queued
**Driver:** human-gated (interactive sessions) — **not** in `.claude/outcome-backlog`
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"The programme" (D3), §"Orchestration: Cloud Run Job, not Composer"
**Spec anchors:** `docs/specs/cli.md`; `docs/specs/run_state.md`; `docs/specs/state.md`; `docs/specs/virtual_environments.md`

## The outcome

The GitHub-activity pipeline runs on a schedule with nobody watching. A container image
holding the smelt binary and `examples/github_activity/` runs as a Cloud Run Job on a
Cloud Scheduler cron, authenticating by workload identity with no key material anywhere.
It is **stateless**: correctness state lives in engine-resident BigQuery tables and
nothing under `.smelt/` survives the container, which is what `state.mode: stateless`
already provides. Each run's report lands in Cloud Logging in a form that can be queried,
a failed run alerts rather than passing silently, and a re-run after a failure is safe.
The research doc's bar — *"it runs unattended against a real dataset and I trust the
numbers"* — is met, with the trust half already established by the spine.

## Success criteria (checkable)

1. **Image.** A reproducible container image carries the smelt binary and the example
   project, pinned by digest, with its build documented and repeatable from the repo.
2. **Identity.** The job authenticates by workload identity against the dogfood project.
   No service-account key is baked into the image, stored in the repo, or held on the
   machine. The dataset-scoped permissions are the minimum the pipeline needs.
3. **Stateless.** The job runs under `state.mode: stateless`; a run leaves nothing under
   `.smelt/` that a subsequent run depends on, verified by running twice on a fresh
   container each time and getting a correct result.
4. **Scheduled.** Cloud Scheduler triggers the Cloud Run Job on a cron matching the
   loader's cadence, with the loader step ordered before the models (by the black-box step
   if `20260906-external-dag-steps` has landed, otherwise by an explicit ordering recorded
   as a known gap).
5. **Observable.** Each run's report reaches Cloud Logging in a queryable form — run id,
   per-model outcome, rows affected, duration, bytes billed — and a query answering
   "did last night's run succeed, and what did it cost?" is written down.
6. **A failure reaches a human.** A failed run raises an alert that arrives somewhere the
   human actually reads. A test failure is deliberately induced once and the alert is
   confirmed to arrive — not assumed from configuration.
7. **Re-run safety.** A run interrupted mid-flight, and a run re-executed after a failure,
   both converge to the same state a clean run would produce — checked against the
   full-refresh oracle, not asserted.
8. **Cost is bounded and known.** The steady-state monthly cost is measured over at least
   a week of real runs and recorded against the budget cap set in the spine's phase 1.
9. **Documented.** A docs-site or `docs/` page describes the deployment end to end, so it
   can be rebuilt from scratch without reverse-engineering the console.

## Out of scope

- Cloud Composer / managed Airflow — argued against in the research doc (≈US$300/month
  floor, buying nothing over smelt's own DAG derivation) and by the build-vs-rent rule.
- Cloud Workflows — the named fallback if cross-target fan-out is ever needed; not now.
- The self-directed scheduler daemon (build-vs-rent puts scheduling execution on the rent
  side).
- Multi-environment promotion, blue/green, or dev/prod target separation.
- Any model, correctness or feature work — the spine and `bigquery-correctness` own those.
- Fixing whatever the unattended runs surface; new defects go to the findings handoff and
  are triaged like any other.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Build the container image: smelt binary plus `examples/github_activity/`, pinned and reproducible from the repo | pending |
| 2 | Workload identity and minimum dataset-scoped permissions; confirm no key material anywhere | pending |
| 3 | Cloud Run Job executing a stateless run; verified correct twice from a fresh container | pending |
| 4 | Cloud Scheduler cron with the loader ordered ahead of the models | pending |
| 5 | Route the run report to Cloud Logging and write the "did it succeed, what did it cost" query | pending |
| 6 | Alerting: induce a real failure and confirm the alert arrives | pending |
| 7 | Re-run safety: interrupt and re-execute, check convergence against the full-refresh oracle | pending |
| 8 | Measure a week of steady-state cost against the budget cap; document the deployment end to end | pending |

## Decision log

- 2026-09-06 (scaffold): **human-gated, out of the backlog.** Every phase provisions or
  changes cloud infrastructure; none of it is executable by a headless loop.
- 2026-09-06 (scaffold): **starts after the spine's criterion 7.** Automating a pipeline
  whose numbers are not yet trusted automates a wrong answer on a cron. The spine's
  full-refresh-oracle criterion is the gate on beginning this outcome.
- 2026-09-06 (scaffold): criterion 6 requires an *induced* failure rather than a
  configured alert. An alert path that has never fired is not evidence, and this is the
  outcome whose whole point is that nobody is watching.

## Blocked

(none)
