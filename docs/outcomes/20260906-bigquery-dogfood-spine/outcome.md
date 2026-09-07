# Outcome: The GitHub-activity pipeline runs on BigQuery and DuckDB, and the numbers agree

**Created:** 2026-09-06
**Status:** in progress
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
2. **Reachable, and only here.** `bq`/`gcloud` are usable from a session against the
   dogfood project via ADC impersonating a dogfood-scoped service account, so the
   credential reaches this project and no other — demonstrated, not assumed: a call
   against a different project of the human's is refused. This requires removing the
   `gcloud`/`bq` entries from the checked-in `deny` list in `.claude/settings.json` (deny
   beats allow, so `settings.local.json` cannot do it). `smelt-bq-test-20260816`'s
   isolation is untouched: the `scripts/bigquery-*.sh` denials and
   `Read(//home/andrew/.config/gcloud-smelt-bq/**)` stay.
3. **Loader.** One scheduled query (or `bq query` step) populates `raw.github_events` from
   `githubarchive.day.2026*`, reproducing `examples/github_activity/sample.sql`
   **verbatim** — same projection, same `MOD(repo.id, 1000) = 0` filter, same
   `_TABLE_SUFFIX` range — landed day-partitioned on `created_at` and trimming partitions
   older than N days. Reproducing that one query is what makes criterion 6 meaningful: two
   targets over different populations are not comparable. It is at-least-once by
   construction and documented as **external to smelt** — smelt's source declaration is
   the contract. Its cost per run is measured and recorded.
4. **DuckDB leg, in CI.** A deterministic Parquet export of the same sample is committed
   *and* reproducibly regenerable, and `examples/github_activity/` runs end-to-end against
   DuckDB with **no live warehouse**: `cargo test -p smelt-cli --test example_diagnostics`
   and `cargo test -p smelt-lsp --test example_workspaces` see zero diagnostics, and the
   four models build. This leg is the cheap oracle; it runs per-PR. Because the upstream
   feed carries **no duplicate event ids** (measured — see decision log), the DuckDB leg
   must reproduce the loader's at-least-once behaviour deliberately, by replaying
   overlapping windows; otherwise `silver.events_deduped` ships with its whole reason for
   existing untested.
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
| 1 | Confirm the public dataset's real schema and sharding, pin the sample as one committed query (`examples/github_activity/sample.sql`), and export it reproducibly to Parquet as the DuckDB leg's input | done |
| 2 | `examples/github_activity/`: smelt.yml, the source declaration, and the four spine models, green end-to-end on DuckDB over the Parquet sample with zero diagnostics and wired into per-PR CI | pending |
| 3 | Provision the dogfood project: dataset with no table expiry, budget alert and cap, ADC for the account, and remove `.claude/settings.json`'s `bq`/`gcloud` deny while leaving the test project's isolation intact — with a rationale note in the commit | planned |
| 4 | Build the loader in the dogfood project: `sample.sql` reproduced verbatim into `raw.github_events`, day-partitioned, day-range-bounded, N-day trimmed; measure and record cost per run | pending |
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
- 2026-09-06 (human): **ADC by impersonation, one project.** The credential is ADC — not
  the existing encrypted-key design — but obtained with
  `--impersonate-service-account=smelt-dogfood@…` rather than as the human's own identity,
  so it reaches the dogfood project and nothing else. No key material is minted or stored.
  The service account holds `roles/bigquery.jobUser` plus `WRITER` on the one dataset,
  deliberately not `roles/bigquery.user`: the test suites need dataset-creation because
  they isolate each run in a fresh dataset, while this pipeline writes to one long-lived
  dataset and never creates its own.
- 2026-09-06 (human): **session access is deliberate; no command-scoping guard.** The four
  blanket `Bash(gcloud *)` / `Bash(bq *)` denies in `.claude/settings.json` are removed
  rather than replaced by a hook admitting only project-scoped invocations. Deny beats
  allow, so the checked-in list had to change either way; the guard was dropped because
  matching command text is not containment (a dynamically built string defeats it) and,
  with the identity above, there is nothing left for it to protect. Isolation of
  `smelt-bq-test-20260816` is unchanged and rests where it actually holds: its credentials
  live in a separate `CLOUDSDK_CONFIG` that stays `Read`-denied, and its self-targeting
  scripts stay denied.
- 2026-09-06 (human): **the Cloud SDK is pinned in `mise.toml`**, as a task plus a computed
  env var rather than a `[tools]` entry — every workflow runs `jdx/mise-action@v2` with no
  arguments, so a `[tools]` pin would pull the ~200MB SDK into all seven CI jobs, none of
  which use it. Mirrors the existing `setup-duckdb` precedent. (SDK 580.0.0 is already
  present on the current box at `~/google-cloud-sdk/bin`, so this is a no-op here and
  earns its keep on the next machine.)
- 2026-09-06 (scaffold): unverified assumption for phase 2 — `githubarchive.day.*` is
  assumed to expose `repo.id` (INT64), `repo.name`, `actor.id`, `created_at` (TIMESTAMP),
  `id`, `type` and a large `payload` STRING. Phase 2 confirms against the live schema
  before the loader is written; no session could query BigQuery at scaffold time.
  **Resolved 2026-09-07** — see the schema entry below.
- 2026-09-07 (human): **DuckDB leg first; provisioning moves behind it.** The phase order
  is inverted so the sample and the example project land before any cloud resource is
  created. Reading the public `githubarchive` dataset needs only an existing billing
  project, so the schema and the fixture were obtainable immediately, while provisioning
  is the slowest and least reversible step in the programme. Ordering it after the DuckDB
  leg means the loader is written against a query already proven to produce usable rows,
  and `examples/github_activity/` earns its per-PR CI coverage whether or not the cloud
  half ever lands. Old phases 1→3 and 2→4, with 2's schema half promoted into the new
  phase 1; `phases/01-plan.md` moved to `phases/03-plan.md` unchanged.
- 2026-09-07: **the four blanket `gcloud`/`bq` denies were removed ahead of phase 3**, not
  as part of it — nothing could be probed without them. Everything phase 3's plan says
  about the change still holds, including that the `scripts/bigquery-*.sh` denials and the
  `Read(//home/andrew/.config/gcloud-smelt-bq/**)` deny stay. `bq version` answers D2:
  BigQuery CLI 2.1.36 runs fine on this box, so the pyOpenSSL failure the older scripts
  work around is not present — but the dogfood path still speaks REST over `curl`, one way
  of talking to BigQuery, as `scripts/bq-dogfood-query.sh`.
- 2026-09-07 (human): **the exploration bills `smelt-bq-test-20260816`.** Interim only, and
  read-only against a public dataset — it creates nothing in the test project. Phase 3
  replaces it with the dogfood project's own credential. The token is the existing
  short-lived one from `scripts/bigquery-auth.sh`; `scripts/bq-dogfood-query.sh` and
  `scripts/bq_dogfood_export.py` consume it from the environment and never print it.
- 2026-09-07: **the real schema of `githubarchive.day`.** Confirmed against
  `day.20260901`. The scaffold's guess was right about `repo.id`/`actor.id` (INTEGER inside
  `repo`/`actor` RECORDs), `repo.name`, `created_at` (TIMESTAMP) and `payload` (STRING),
  and wrong in three ways that change the loader:
  - **`id` is STRING, not an integer.** The dedup key is textual.
  - **The table is neither partitioned nor clustered.** `day` is a set of daily-*sharded*
    tables. Pruning is `_TABLE_SUFFIX`, not partition elimination — so the loader's own
    `raw.github_events` is where day-partitioning first exists.
  - **A bare `day.*` wildcard fails outright**: the dataset also holds views (`yesterday`,
    …) and BigQuery refuses `Views cannot be queried through prefix`. The prefix must be
    `day.2026*`, which excludes them.
  Two further columns exist and are worth having: `public` (BOOL) and `org` (RECORD).
  Volume: ~637k rows / ~421 MB per day.
- 2026-09-07 (human): **the sample is 30 days at `MOD(repo.id, 1000) = 0`.** Widening the
  modulus was measured and rejected: over one week, MOD 1000 yields 3 renamed repos and
  MOD 100 yields 7 — renames run about one per thousand repo-weeks, so a *longer window*
  buys the rename stream that a *wider sample* does not. 30 days at MOD 1000 gives 64,313
  rows / 4,491 actors / 5,016 repos / **34 renamed repos** in 1.1 MB of Parquet, for 6.3 GB
  scanned (~US$0.03). Pinned as `examples/github_activity/sample.sql`; regenerate with
  `examples/github_activity/refresh_sample.sh`.
- 2026-09-07: **the upstream feed has no duplicate event ids** — 64,313 rows, 64,313
  distinct `id`s. The at-least-once property the spine dedups against belongs entirely to
  the *loader's* overlapping windows, not to GitHub Archive. Criterion 4 is amended: the
  DuckDB leg has to replay overlapping windows on purpose or `silver.events_deduped` is
  never exercised.
- 2026-09-07 (human): **the loader redelivers on purpose.** Since the upstream feed carries
  no duplicates of its own, the black box supplies the at-least-once behaviour it is
  declared to have: each day's load re-appends a deterministic slice of the **previous**
  day's rows. This replaces an earlier overlapping-window shape, which had an escape hatch
  — both copies landing in one load and one partition, removable by a partition-local
  `QUALIFY` with zero lookback, exercising dedup only in its most trivial case. A day-old
  duplicate is outside the current window, so only a real SQL-derived lookback catches it.
  Constraints this puts on phase 4: the rule is **deterministic** (`MOD(CAST(id AS
  BIGINT), 50) = 0`, which evaluates identically on DuckDB and GoogleSQL — not `RAND()`,
  not `FARM_FINGERPRINT`), it is **2%** rather than 0.1% (0.1% is ~2 rows/day here, thin
  enough for a wrong lookback to pass on luck; 2% matches `examples/web_analytics/` so the
  two examples compare), it draws from **exactly** the previous day so the required
  lookback is exactly two days and an error either way fails, and it is pinned contract
  the loader reproduces verbatim — like `sample.sql`, two legs redelivering differently
  are not comparable. Note that `mutation_profile.lateness` is *not* the mechanism: it is
  orchestration-only and never widens a scan.
- 2026-09-07: **the sample is skewed, and that is a finding rather than a defect.** 92% of
  events are `PushEvent`, the median repo has one event, and one bot repo has 527.
  `MOD(repo.id, 1000)` is uniform over repo *ids*, and recent ids are dominated by bulk
  repo creation. Sessionization and dedup are unaffected; any mart reading like a
  leaderboard will look strange, and should be read as a property of the sample.
  Recorded now so a later reader does not diagnose it as a pipeline bug.
- 2026-09-07: **an early tension-1 datum, ahead of criterion 9.** Of the renames visible in
  the 7-day probe, one is not a rename at all in the usual sense:
  `mikiKG45/noob-devops-project` → `guslariR45/noob-devops-project` — same `repo.id`, same
  trailing name, different *owner*. Keyed succession over `repo.id` sees an ordinary
  attribute change; a grammar keyed on the owner/name pair sees a discontinuity. Phase 9
  should probe this case specifically, not only plain renames.

## Blocked

(none)
