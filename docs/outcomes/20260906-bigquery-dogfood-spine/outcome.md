# Outcome: The GitHub-activity pipeline runs on BigQuery and DuckDB, and the numbers agree

**Created:** 2026-09-06
**Status:** queued
**Driver:** human-gated (interactive sessions) — **not** in `.claude/outcome-backlog`
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"The programme" (D0, D1), §"The example project"
**Spec anchors:** `docs/specs/sources.md`; `docs/specs/multi_backend.md`; `docs/specs/incremental_models.md` §"The equivalence invariant"; `docs/specs/smelt_yml.md`; `docs/specs/run_state.md`; `docs/specs/state.md`

## The outcome

A dedicated, budget-capped GCP project holds `raw.github_events`: a day-partitioned,
column-pruned, retention-trimmed copy of a **stable 0.1% sample** of GitHub Archive,
produced by a scheduled BigQuery query that smelt orders but does not author. Four
models — `bronze.events`, `silver.events_deduped`, `silver.actor_sessions` and one mart
that makes their output visible — live in `examples/github_activity/` and run to
completion **on both targets**: incrementally against BigQuery over successive windows,
and against DuckDB in ordinary CI over a Parquet export of the identical sample. The two
targets produce the same answers, every incremental state matches a full-refresh oracle,
and every divergence between them is registered rather than tolerated. The defects the
live run surfaces are written down as a punch-list rather than fixed here.

## Success criteria (checkable)

1. **Provisioned.** A dedicated GCP project (not `smelt-bq-test-20260816`) with a dataset
   carrying **no** default table expiration, a budget alert, and a documented monthly cap.
   `docs/research/20260816-bigquery-backend.md`'s provisioning decisions are followed
   except where this outcome's decision log records a departure.
2. **Reachable, deliberately.** `bq`/`gcloud` are usable from a session against the
   dogfood project via ADC. This requires narrowing the checked-in `deny` list in
   `.claude/settings.json` (deny beats allow, so `settings.local.json` cannot do it).
   The narrowing keeps `smelt-bq-test-20260816`'s isolation intact: the
   `scripts/bigquery-*.sh` denials and `Read(//home/andrew/.config/gcloud-smelt-bq/**)`
   stay, and the commit carries a rationale note naming the risk accepted.
3. **Loader.** One scheduled query (or `bq query` step) populates `raw.github_events` from
   `githubarchive.day.*`: day-partitioned on `created_at`, projecting only the columns the
   spine consumes (`payload` pruned or narrowed), filtered to `MOD(repo.id, 1000) = 0`,
   bounded to a declared day range, and trimming partitions older than N days. It is
   at-least-once by construction and documented as **external to smelt** — smelt's source
   declaration is the contract. Its cost per run is measured and recorded.
4. **DuckDB leg, in CI.** A deterministic Parquet export of the same sample is committed
   or reproducibly generated, and `examples/github_activity/` runs end-to-end against
   DuckDB with **no live warehouse**: `cargo test -p smelt-cli --test example_diagnostics`
   and `cargo test -p smelt-lsp --test example_workspaces` see zero diagnostics, and the
   four models build. This leg is the cheap oracle; it runs per-PR.
5. **BigQuery leg, live.** The same four models compile and run against the dogfood
   project — a full refresh, then **at least three consecutive incremental windows** —
   with the run report from W2 captured for each.
6. **The two targets agree.** A dual-target parity check compares each model's output
   between DuckDB (over the Parquet sample) and BigQuery (over the same rows): equal, or
   the difference is registered as a named divergence with a reason. An unregistered
   difference fails.
7. **The numbers are trustworthy.** After each incremental window, each model's state
   equals a full refresh over the inputs seen so far, on **both** targets — the
   equivalence invariant checked on a real pipeline rather than a generated recipe.
8. **Evidence banked.** `docs/handoffs/2026-XX-XX-github-activity-findings.md` lists every
   defect, divergence, missing emission verdict, and unsupported construct the live runs
   surfaced, each with the model and statement that provoked it. This document is the
   input to `docs/outcomes/20260906-bigquery-correctness`, and the requirements it names
   are the input to the `external-dag-steps` and `trimmed-history-sources` outcomes.
9. **Tension 1 probed, not solved.** The real `(repo.id, repo.name)` rename stream is run
   against the keyed-succession grammar as specced and the exact refusal (or acceptance)
   is recorded in `docs/outcomes/20260906-scd2-keyed-succession`'s decision log. No
   grammar change is made here.
10. **Gates green.** `bash .claude/scripts/verify-phase.sh` passes; no ratchet lowered.

## Out of scope

- **SCD2 in the first pass.** `silver.repo_naming` and `marts.naming_history` are not
  built here (human decision of 2026-09-06 — see decision log). Criterion 9 keeps the
  tension probed without building the grain.
- The full sketch in the research doc: the silver fan-out (`push_events`, `pr_events`,
  `issue_events`, `star_events`), `manual.repo_watchlist` and the `ColumnScopedMerge`
  shape it reaches, `silver.actor_naming`, the gold and mart layers beyond the one mart.
  These arrive once the spine is trustworthy; widening the sample is the same gesture.
- **Fixing** anything the live run surfaces, beyond what is needed to make a run complete
  at all. Fixes belong to `20260906-bigquery-correctness`.
- Unattended scheduling — `20260906-bigquery-unattended` owns Cloud Run Job, Scheduler,
  workload identity and log routing.
- The `produced_by:` declaration and the trimmed-history bound as *smelt features* —
  owned by `20260906-external-dag-steps` and `20260906-trimmed-history-sources`. Here the
  loader is external by convention and the retention bound is a fact recorded in prose.
- Everything in the research doc's §"Out of scope" (dbt importer, slim CI, packages,
  auth, other backends, the scheduler daemon).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Provision the dogfood project: dataset with no table expiry, budget alert and cap, ADC for the account, and narrow `.claude/settings.json`'s `bq`/`gcloud` deny so it scopes to the dogfood project while leaving the test project's isolation intact — with a rationale note in the commit | pending |
| 2 | Confirm the public dataset's real schema and partitioning, then build the loader: sampled on `MOD(repo.id, 1000)`, column-pruned, day-partitioned, day-range-bounded, N-day trimmed; measure and record cost per run | pending |
| 3 | Export the identical sample to Parquet, reproducibly, as the DuckDB leg's input | pending |
| 4 | `examples/github_activity/`: smelt.yml, the source declaration, and the four spine models, green end-to-end on DuckDB over the Parquet sample with zero diagnostics and wired into per-PR CI | pending |
| 5 | First live BigQuery run: full refresh of the same four models against the dogfood dataset; record every compile refusal and runtime failure rather than fixing them in place | pending |
| 6 | Three or more consecutive incremental windows on BigQuery, run reports captured, frontier and engine-resident state inspected between runs | pending |
| 7 | Dual-target parity: compare every model's output between DuckDB and BigQuery over the same rows; register each difference with a reason or fail | pending |
| 8 | Trust the numbers: full-refresh oracle vs incremental state after each window, on both targets | pending |
| 9 | Probe tension 1: run the real rename stream against the keyed-succession grammar and record the verdict in the scd2 outcome's decision log | pending |
| 10 | Bank the evidence: the findings handoff, the punch-list handed to `bigquery-correctness`, and the requirements handed to the two feature outcomes | pending |

## Decision log

- 2026-09-06 (scaffold, human): **split by driver.** This programme is human-gated —
  provisioning, credentials and live runs cannot be executed by a headless loop — so this
  outcome and `20260906-bigquery-unattended` stay **out** of `.claude/outcome-backlog`,
  while the three loop-grindable outcomes it generates are queued in it.
- 2026-09-06 (human): **SCD2 leaves the first pass.** The research doc argues tension 1
  should be confronted in the spine; the human's call is that live evidence sooner beats
  grammar evidence earlier, and correctness should be dealt with cheaply before breadth.
  The tension is not dropped — criterion 9 probes it once the pipeline is live, and its
  finding still reaches `scd2-keyed-succession` before that outcome's classifier phase.
- 2026-09-06 (human): **a small subset first, widened later.** The human asked for a cheap
  slice. Sampling is `MOD(repo.id, 1000) = 0` rather than a `repo.name` prefix: a name
  prefix is unstable under rename, so a renamed repo would silently leave the sample —
  corrupting exactly the rename history the later SCD2 work depends on. `repo.id` is
  stable, the sample is uniform, and widening is a one-token change to the modulus.
  Cost note: BigQuery bills bytes scanned, so the sample filter alone does not reduce the
  loader's bill — pruning `payload` and bounding the day range is what does.
- 2026-09-06 (human): **both targets, always.** DuckDB is not a fallback but the cheap
  oracle: the same four models over the same rows on both engines make a dual-target diff
  (criterion 6) the least expensive way to find the class of defect the research doc says
  only live runs catch. It also gives `examples/github_activity/` real CI coverage, which
  the research doc anticipated.
- 2026-09-06 (scaffold): **ADC, not the encrypted-key pattern.** The human chose plain ADC
  scoped to the dogfood project over extending the existing non-ambient design. Consequence
  discovered while scaffolding: `.claude/settings.json` hard-denies `Bash(bq *)` and
  `Bash(gcloud *)`, and deny beats allow, so this cannot be done in `settings.local.json` —
  phase 1 must narrow the checked-in list, which is why it is called out as its own task
  with a rationale requirement rather than treated as configuration.
- 2026-09-06 (scaffold): unverified assumption for phase 2 — `githubarchive.day.*` is
  assumed to expose `repo.id` (INT64), `repo.name`, `actor.id`, `created_at` (TIMESTAMP),
  `id`, `type` and a large `payload` STRING. Phase 2 confirms against the live schema
  before the loader is written; no session could query BigQuery at scaffold time.

## Blocked

(none)
