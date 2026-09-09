# Outcome: The GitHub-activity pipeline runs on BigQuery and DuckDB, and the numbers agree

**Created:** 2026-09-06
**Status:** in progress
**Driver:** split. Phases 2–4, 6, 8 and 9 are loop-grindable (no warehouse, no credentials)
and this outcome sits in `.claude/outcome-backlog` for them. Phase 5 needs a human-minted
BigQuery token for one fixture regeneration (see "## Blocked"); phases 7, 10–14 and 16 are
**human-gated** — they provision cloud resources and run live BigQuery, which a headless
loop cannot do, so those phases must emit `<<PHASE_BLOCKED>>` rather than attempt it.
Phase 15 banks the DuckDB half of the evidence and is loop-grindable.
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"The programme" (D0, D1), §"The example project"
**Spec anchors:** `docs/specs/sources.md`; `docs/specs/multi_backend.md`; `docs/specs/incremental_models.md` §"The equivalence invariant"; `docs/specs/smelt_yml.md`; `docs/specs/run_state.md`; `docs/specs/state.md`

## The outcome

A dedicated, budget-capped GCP project holds `raw.github_events`: a day-partitioned,
retention-trimmed copy of a **stable 0.1% sample** of GitHub Archive, produced by a
scheduled BigQuery query that smelt orders but does not author. A bronze→silver→gold→mart
pipeline lives in `examples/github_activity/` — dedup over an at-least-once feed,
gap-based sessionization, keyed succession over the real rename stream, typed payload
extraction, and the marts that make all of it visible — and runs to completion **on both
targets**: incrementally against BigQuery over successive windows, and against DuckDB in
ordinary CI over a Parquet export of the identical sample. The two targets produce the same
answers, every incremental state matches a full-refresh oracle, and every divergence
between them is registered rather than tolerated. The defects the live run surfaces are
written down as a punch-list rather than fixed here.

The DuckDB half is built and trusted **first**, in full. Every model, and the equivalence
invariant over all of them, is green with no warehouse before a single cloud resource
exists — so the live run is a test of the *backend*, not of the models.

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
4. **DuckDB leg, in CI — the whole pipeline, not a spine.** A deterministic Parquet export
   of the same sample is committed *and* reproducibly regenerable, and
   `examples/github_activity/` runs end-to-end against DuckDB with **no live warehouse**:
   `cargo test -p smelt-cli --test example_diagnostics` and
   `cargo test -p smelt-lsp --test example_workspaces` see zero diagnostics, and every
   model builds — bronze, the dedup, sessionization, both succession models, the typed
   fan-out, gold and the marts. This leg is the cheap oracle; it runs per-PR. Because the
   upstream feed carries **no duplicate event ids** (measured — see decision log), the
   loader redelivers a deterministic 2% slice of the previous day on purpose; otherwise
   `silver.events_deduped` ships with its whole reason for existing untested.
5. **BigQuery leg, live.** The same models compile and run against the dogfood
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
9. **Succession exercised on the real rename stream.** `silver.repo_naming` is recognised
   as the succession grain from its SQL shape alone and maintained over the fixture's 34
   renamed repos — including the two repo *names* reused across different `repo_id`s and
   the owner-change case where id and trailing name both survive. `silver.actor_naming` is
   a second instance on a different key and clock. **Both** partition postures are covered:
   the source is event-time-partitioned, so the deliberate previous-day redelivery lands in
   a *closed* partition and drives the append-only probe's late-arrival classification,
   while a loader-stamped `ingested_date` makes the arrival-partitioned posture reachable
   from the same pipeline. `SuccessionClockTie`'s "identical rows are a redelivery and fold
   once" leg is exercised by that same redelivery. Every refusal or acceptance that
   surprises is recorded in `docs/outcomes/20260906-scd2-keyed-succession`'s decision log;
   no grammar change is made here.
10. **Gates green.** `bash .claude/scripts/verify-phase.sh` passes; no ratchet lowered.

## Out of scope

- **`manual.repo_watchlist`.** The research doc wanted it because `Technique::
  ColumnScopedMerge` had no reachable shipped shape; `docs/TODO.md` records that gap
  **resolved 2026-08-09**, so the model's original motivation is gone. `gold.events_enriched`
  is built instead, and is a real-pipeline instance of the same `LEFT JOIN`-against-a
  `unique_key`-declaring-dimension shape.
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
| 2 | `examples/github_activity/`: smelt.yml, the source declaration, and the four spine models, green end-to-end on DuckDB over the Parquet sample with zero diagnostics and wired into per-PR CI | done |
| 3 | Succession on the real rename stream: `silver.repo_naming`, `silver.actor_naming` and `marts.naming_history`, exercising **both** partition postures and the redelivery-folds-once leg | done |
| 4 | The payload-independent widening: `gold.repo_dim`, `gold.events_enriched` (the `LEFT JOIN`-against-a-`unique_key`-dimension shape), `gold.repo_activity_daily`, `marts.repo_leaderboard`, `marts.star_growth` | done |
| 5 | Re-pin `sample.sql` with `payload`, regenerate the fixture, and build the typed silver fan-out (`push_events`, `pr_events`, `issue_events`, `star_events`) | in progress |
| 6 | Trust the DuckDB numbers: full-refresh oracle vs incremental state across the whole widened model set, banked before any cloud spend | done |
| 7 | Provision the dogfood project: dataset with no table expiry, budget alert and cap, ADC for the account, and remove `.claude/settings.json`'s `bq`/`gcloud` deny while leaving the test project's isolation intact — with a rationale note in the commit | blocked |
| 8 | Settle the DuckDB half of criterion 7: characterise and bound `gold.events_enriched`'s per-window enrichment staleness, un-`#[ignore]` `every_window_matches_the_full_refresh_oracle`, and hand the derivation gap to `bigquery-correctness` | done |
| 9 | Author the loader artifact with no cloud: `scripts/bq-dogfood-loader.sh` derives the load SQL *from* `sample.sql` (rolling `_TABLE_SUFFIX` day range, `ingested_date` stamp, deliberate previous-day redelivery slice) plus the `raw.github_events` DDL and the N-day retention bound, gated by a per-PR `--emit-sql` test that proves the projection and filter are byte-identical to `sample.sql` | done |
| 10 | Deploy the loader in the dogfood project and run it: `raw.github_events` created day-partitioned, at least two days loaded, retention verified, cost per run measured and recorded | blocked |
| 11 | First live BigQuery run: full refresh of the whole model set against the dogfood dataset; record every compile refusal and runtime failure rather than fixing them in place | blocked |
| 12 | Three or more consecutive incremental windows on BigQuery, run reports captured, frontier and engine-resident state inspected between runs | blocked |
| 13 | Dual-target parity: compare every model's output between DuckDB and BigQuery over the same rows; register each difference with a reason or fail | blocked |
| 14 | Trust the numbers on both targets: full-refresh oracle vs incremental state after each window | blocked |
| 15 | Bank the DuckDB-half evidence now: `docs/handoffs/2026-09-08-github-activity-findings.md` carrying the four measured root causes, the five registered divergences and the loader/retention requirements, so the three downstream outcomes' harvest phases can proceed without live BigQuery | done |
| 16 | Extend the handoff with the live-BigQuery findings: every compile refusal, runtime failure and cross-target divergence the live runs surfaced, plus the final punch-list | blocked |

## Decision log

- 2026-09-09 (phase 5, token-gated half): **the `payload` re-pin is population-identical,
  and it cost twice the estimate.** A human minted a `bigquery-auth.sh` token, `sample.sql`
  was re-pinned to project `payload` (the raw JSON string, nothing else added
  speculatively), and `refresh_sample.sh` regenerated the fixture: 64,313 rows, 5.8 MB
  Parquet, up from 1.1 MB. Two things worth pinning down rather than assuming:
  (a) the nine pre-existing columns are row-for-row equal to the previous committed fixture
  in **both** directions (`EXCEPT ALL`, 0 and 0), so the fan-out is purely additive and no
  existing model's numbers — or the phase 8 divergence registry bounds measured over
  them — are disturbed by the re-pin; (b) BigQuery billed **24.3 GB, ~US$0.12**, double
  the ~12 GB / US$0.06 this log estimated on 2026-09-08, because `payload` dominates the
  scan by more than the estimate assumed. The 5.8 MB fixture is far inside the size that
  would have triggered the standing "shorten the day range rather than narrow the payload"
  instruction, so the pinned 30-day `_TABLE_SUFFIX` range is unchanged. The loader's
  byte-identity gate (`github_activity_loader`) passes unmodified with `payload` in the
  projection — the derivation from `sample.sql` carried it through with no edit.

- 2026-09-09 (bookkeeping): **phase 6 corrected from `blocked` to `done`.** Its own
  "## Blocked" entry has said "Resolved by phase 8" since 2026-09-08 — the centrepiece
  `every_window_matches_the_full_refresh_oracle` is un-`#[ignore]`d and green over all 30
  windows — but the phases table was never updated to match, leaving a row that read as
  outstanding work when its subject was finished. No work was done for this; the row was
  wrong, not the phase.

- 2026-09-08 (terminal): **outcome marked `blocked` rather than `done`.** Judged the success
  criteria against the phase summaries: the DuckDB leg is substantially delivered and its
  evidence is banked in `docs/handoffs/2026-09-08-github-activity-findings.md`, but criteria
  1, 2, 5 and 6 -- provisioning, scoped reachability, the live BigQuery run and dual-target
  parity -- cannot be reached without a GCP project and credential. No `pending` row remains
  and no `blocked` row is unblockable by a headless step, so the outcome cannot progress
  further under the loop. The two human actions that unblock it, and the order to take them
  in, are in this file's "## Blocked" terminal entry.

- 2026-09-08 (phase 10 plan): **phases 10-14 and 16 marked `blocked` in one pass, and the
  evidence-banking phase split so the loop has real work.** All five live phases share one
  gate, already documented in this file's header and in phase 7's "## Blocked" entry: they
  need a provisioned GCP project and a credential that does not exist (re-verified this
  iteration: `gcloud auth list` -> "No credentialed accounts", no
  `~/.config/gcloud/application_default_credentials.json`). Blocking them one iteration at a
  time would burn five planner contexts to learn the same fact five times. Against that, the
  DuckDB half has produced substantial, *bankable* evidence that three downstream outcomes are
  explicitly waiting on -- `bigquery-correctness` phase 2 ("read the spine's findings handoff
  and rewrite the remaining phases from it"), and `external-dag-steps` / `trimmed-history-
  sources` phase 2 ("written by phase 1's planner from the spine's requirements") are all
  parked on a document that does not exist yet. Old phase 15 (bank *all* the evidence) is
  therefore split: new phase 15 banks the DuckDB-half findings now and is loop-grindable; new
  phase 16 extends the same document with the live-run findings and inherits the live gate.
  Criterion 8 is unchanged and is met by the two together -- nothing left this outcome.

- 2026-09-08 (phase 9 implement): **the loader's own retention bound (45 days,
  `partition_expiration_days`) is a different number from the pre-existing
  `retention: '90 days'` field already declared on both
  `models/sources/raw/github_events{,_arrival}.yml`.** That field's own comment claims it
  "matches the loader's own N-day trim (criterion 3)" — written before this phase measured
  anything. `retention:` is parsed into `smelt-core`'s `SourceDefinition` today but consumed
  by no maintenance logic (grep confirmed: no reader outside test fixtures) — it is inert,
  scaffolding for `20260906-trimmed-history-sources`, which owns making it a real smelt
  feature. Left uncorrected here (out of this phase's task list, and this outcome's own
  "Out of scope" assigns the trimmed-history bound as a *smelt feature* to that other
  outcome) — but the comment's claim is now false and should be fixed by whichever outcome
  next touches those two files, to avoid a future reader trusting the 90 rather than the
  45 that `scripts/bq-dogfood-loader.sh --emit-ddl` and `README.md` actually declare.
- 2026-09-08 (phase 9 plan): **reshape — the loader phase is split into an authoring half
  the loop can do and a deploy half it cannot; old 10–14 become 11–15.** As written, phase 9
  bundled "reproduce `sample.sql` verbatim" (pure text/SQL authoring, no cloud) with "land it
  in the dogfood dataset and measure cost per run" (needs the project phase 7 is blocked on —
  re-checked this iteration: `gcloud` reports "No credentialed accounts", so nothing has been
  provisioned). Blocking the whole thing would strand the verbatim-reproduction work that
  criterion 3 — and through it criterion 6's comparability — actually turns on, and would
  leave the human's deploy step improvising the query at the console. New phase 9 makes
  "verbatim" mechanical rather than aspirational: the loader *derives* its SQL from
  `sample.sql` and a per-PR test asserts the projection and `MOD(repo.id, 1000)` filter come
  through byte-identical, with only the `_TABLE_SUFFIX` range parameterised. Nothing left the
  outcome — new phase 10 carries the deploy, the retention check and the cost measurement,
  and stays human-gated.

- 2026-09-08 (phase 8 implement): **criterion 7's DuckDB half is settled — the phase 6
  "self-heals" claim was wrong, and two more previously-unknown divergences surfaced.**
  `every_window_deep_sweep` (new, `#[ignore]`d measurement test in `github_activity_oracle.rs`)
  checked all 30 windows and found the `gold_events_enriched` stale-row count strictly
  non-decreasing (1 → 39 across the fixture) — it never heals, contradicting phase 6's
  "manual rerun" claim, which turns out to have been a row-count check, not a content check.
  The registered bound is therefore non-fabrication (every stale `current_repo_name` is a
  name the repo genuinely held earlier, per `StaleButHistoricallyValid`), not convergence —
  there is no N to converge within. The same sweep also found `silver_actor_sessions` and
  `marts_daily_active_contributors` diverging, unregistered, from two further distinct root
  causes: (a) `compute_calendar_windows` (`crates/smelt-runtime/src/windowing.rs`) applies
  the Form-B forward-reach rebase only at a multi-day invocation's outer edges, never an
  interior chunk boundary, so **the full-refresh oracle itself under-counts** a cross-midnight
  session inside a wide `--full-refresh` (the incremental leg is correct, confirmed against a
  from-scratch raw-SQL recomputation) — not specific to `--full-refresh`, any wide
  single-invocation Form-B materialization is affected; (b) the downstream mart has no
  rebase of its own and never revisits an already-written partition, so it never learns when
  `actor_sessions`'s own correct rebase rewrites an earlier partition — a third instance of
  the same missing-repair-edge shape as `gold_events_enriched`'s finding, triggered by an
  ordinary self-rebase rather than a renamed dimension. `DivergenceEntry`/`check_bound`
  generalised to a 3-shape `Bound` enum (`FoldEquality`, `StaleButHistoricallyValid`,
  `MonotoneDivergence` with a `behind_side` since the two new entries diverge in *opposite*
  directions) to cover all five entries. `every_window_matches_the_full_refresh_oracle`
  un-ignored and promoted to check every one of the 30 windows (measured 108s, under the
  5-minute budget, so the first-10-plus-final sampling was dropped). All four measured root
  causes (two succession-tie folds, the enrichment freeze, the oracle windowing gap, the mart
  repair gap) are handed to `bigquery-correctness` as criterion-8 findings — none fixed here.
  Full write-up: `phases/08-summary.md`.

- 2026-09-08 (phase 8 plan): **reshape — a new loop-grindable phase 8 is inserted ahead of
  every cloud phase; old 8–13 become 9–14.** Phase 7 blocked on human provisioning and
  nothing downstream of it (the loader, the live runs, the parity check, the evidence bank)
  is executable without a GCP identity — verified this iteration, `gcloud auth list` still
  reports "No credentialed accounts". Declaring the whole outcome blocked there would defer
  work that **is** runnable and **does** serve a success criterion: criterion 7's DuckDB
  half is not met, because phase 6's centrepiece
  (`every_window_matches_the_full_refresh_oracle`) is `#[ignore]`d on an uncharacterised
  `gold.events_enriched` divergence. Per this process's own rule — work serving the success
  criteria is never deferred out — that becomes phase 8 rather than leaving the outcome. It
  takes option 2 then option 1 of phase 6's blocked entry, in that order: widen the sweep to
  every day first (cheap, and it settles whether the staleness is bounded or recurring)
  *before* spending effort root-causing, so the bound that gets registered is measured rather
  than invented. Explicitly **not** in the phase: fixing smelt's derivation gap — this
  outcome's "## Out of scope" assigns fixes to `20260906-bigquery-correctness`, so phase 8
  characterises, bounds, registers and hands off. Phase 6 stays `blocked` (its own budget is
  spent and its infrastructure landed); phase 8 is the row that finishes its subject.

- 2026-09-08 (phase 6 plan): **no reshape; the phase is sharpened rather than moved.**
  Phase 4's summary surfaced one finding (`RepairKeysNotDiscoverable` for
  `gold.repo_dim`'s mutation sensitivity) and it is already owned — criterion 8's handoff,
  banked by phase 13 — so no row is added, split or dropped. Two things the plan pins that
  the phase title left open: (a) criterion 7 says *after each* window, and today's check
  (`full_refresh_matches_incremental_replay`) compares only **at the end** and only by
  **row count**, so the phase's real content is a per-window, row-for-row oracle over every
  materialised relation, discovered rather than hardcoded; (b) the two succession
  divergences stop being hardcoded 139/145 deltas and become bounded registry entries —
  after folding both sides on `(key, clock)` the multisets must be *equal* — which is the
  honest statement of phase 3's finding and the same registry shape criterion 6 (phase 11,
  dual-target) will need. Measured while planning: the existing 30-day replay + one full
  refresh runs in 36s, so ~30 additional growing-input full refreshes are affordable
  per-PR; the plan sets a 5-minute budget with an explicit, recorded fallback rather than
  letting windows be dropped quietly. The harness is extracted to a shared test module
  because `github_activity_replay.rs` is 951 lines against the 1000-line default cap.

- 2026-09-08 (phase 4 implement): **measured, not assumed: NO maintenance cell is derived
  for `gold.repo_dim`'s mutation sensitivity at all — a stronger gap than "wrong
  technique".** `smelt explain gold.events_enriched --json` shows no
  `UpstreamMutation(gold.repo_dim)` cell; instead a `RepairKeysNotDiscoverable { source:
  "gold.repo_dim", why: "model has no proven grain and no declared unique key" }` refusal.
  Root cause, read from `crates/smelt-logical/src/maintenance/derive/model_edge.rs::
  append_model_edge_cells`: the key-addressed route (the only one open to a **clockless**
  upstream model) requires the **downstream's own** declared `unique_key` to scope the
  recompute (`admit_key_addressed_recompute`'s `declared_unique_key` parameter), and
  `gold.events_enriched` is `grain: partition`, which has no top-level `unique_key:` slot by
  construction — the clock-based route also does not apply since `gold.repo_dim` declares no
  `timeseries:`. So a `grain: partition` enrichment reading a clockless keyed-model dimension
  has **no reachable route** in the current derivation, independent of whether the dimension
  is itself cleanly classified (`gold.repo_dim`'s own `delta_signature` is `keyed_upsert` over
  `["repo_id"]`, confirmed via its own `smelt explain --json`). Concretely: renaming a repo
  today does not re-derive `current_repo_name` on `gold.events_enriched`'s already-written
  rows through any tracked technique — silent staleness, not a loud refusal at run time (the
  refusal only surfaces via `explain`, not `run`). Characterised (not fixed) by
  `events_enriched_dimension_mutation_cell_technique` in `crates/smelt-cli/tests/
  github_activity_replay.rs`; handed to `docs/outcomes/20260906-bigquery-correctness` as a
  criterion-8 finding. Also measured: a self-join of two CTEs over the same upstream model
  resolves to no classifiable `OutputDelta` at all for `own_output_delta_shape`, while a
  plain `GROUP BY <declared_unique_key>` aggregate over the same source resolves cleanly to
  `KeyedUpsert` — `gold.repo_dim`'s SQL uses the latter shape for exactly this reason, though
  it does not change the finding above (the refusal traces to `events_enriched`'s own grain,
  not to `repo_dim`'s shape).
- 2026-09-08 (phase 4 implement): **the sample's measured skew moved between planning and
  implementation.** The 2026-09-07 decision log entry recorded "one bot repo has 527" events;
  the committed 30-day fixture (measured now, via `marts.repo_leaderboard`) actually tops out
  at repo_id 1331137000 (`mosleyamanda283/eltuxy`) with **1,750** events. `README.md` and
  `repo_leaderboard_top_repo_is_the_bot_repo` use the current measured figure; the 527 number
  was evidently from an earlier, smaller probe of the same sample query and was never
  load-bearing (no code or test depended on it before this phase).
- 2026-09-08 (phase 4 plan): **the `payload` re-pin is split out of phase 4 and blocked.**
  Phase 4 as written opened with re-pinning `sample.sql` to project `payload` and
  regenerating the fixture — which is a live BigQuery query, and the credential path is a
  1-hour token that only a human can mint (`scripts/bigquery-auth.sh` prompts for the
  passphrase protecting the encrypted key; probed this session, no valid token exists and
  `Read(//home/andrew/.config/gcloud-smelt-bq/**)` is denied besides). A headless loop
  cannot do it, so it becomes its own row (new phase 5, `blocked`) together with the only
  work that actually depends on it — the typed silver fan-out, whose whole subject is
  extraction *from JSON*. Everything else the old phase 4 named is payload-independent and
  stays as phase 4: `gold.events_enriched`, `gold.repo_activity_daily` and the remaining
  marts. Nothing is deferred out of the outcome; criterion 4's fan-out clause is still
  owned by a row, it is just a row a human has to unblock. Re-fetching the payloads from
  gharchive.org's public hourly files instead was considered and rejected: reproducing the
  30-day sample that way is tens of GB of download, and it would no longer be `sample.sql`
  run verbatim — which is the whole basis of criterion 3's comparability.
- 2026-09-08 (phase 4 plan): **the enrichment dimension is a smelt model, and its derived
  technique is measured rather than assumed.** `manual.repo_watchlist` is out of scope, so
  the `unique_key`-declaring dimension `gold.events_enriched` left-joins is a new
  `gold.repo_dim` (one row per repo, current name from `silver.repo_naming`). The shipped
  `ColumnScopedMerge` shape (`ValueEnrichedRecipe` in `smelt-maintenance-testkit`) is
  proven over a declared **source** with `mutation_profile: mutable_snapshot` and
  `unique_key:`; whether an upstream *model* presents the same mutation-sensitivity facts
  is not established anywhere in the tree. Phase 4 therefore asserts the technique the
  plan actually derives via `smelt explain --json` and records it. A verdict other than
  `ColumnScopedMerge` is a **finding for criterion 8's handoff**, not a phase failure —
  this outcome builds and records, it does not fix derivation.
- 2026-09-08 (phase 4 plan): **`marts.star_growth` is thin on purpose.** The fixture holds
  47 `WatchEvent`s over 30 days (measured). That is enough to pin exact counts in a test and
  not enough to look like an analytics product — the same skew finding as the leaderboard
  (92% `PushEvent`), and `repo_leaderboard` is built anyway because the research doc's full
  sketch names it and a mart that reads oddly under a known-skewed sample is still a real
  consumer of `gold.repo_activity_daily`.

- 2026-09-06 (scaffold, human): **split by driver.** This programme is human-gated —
  provisioning, credentials and live runs cannot be executed by a headless loop — so this
  outcome and `20260906-bigquery-unattended` stay **out** of `.claude/outcome-backlog`,
  while the three loop-grindable outcomes it generates are queued in it.
- 2026-09-06 (human): **SCD2 leaves the first pass.** The research doc argues tension 1
  should be confronted in the spine; the human's call is that live evidence sooner beats
  grammar evidence earlier, and correctness should be dealt with cheaply before breadth.
  The tension is not dropped — criterion 9 probes it once the pipeline is live, and its
  finding still reaches `scd2-keyed-succession` before that outcome's classifier phase.
  **Superseded 2026-09-08** — see the reshape entry below.
- 2026-09-08 (human): **the whole DuckDB pipeline first; all of GCP last.** The premise of
  the 2026-09-06 decision was that succession would have to be *built* to be probed. It no
  longer does: `20260906-scd2-keyed-succession` shipped the grain to `main` in the interim
  (example workspace, twelve refusal cases in `examples/broken/`, the append-only posture
  probe, `smelt explain` rendering, spec divergences closed), so `silver.repo_naming` is
  now a shape that either works or refuses loudly. Building it is cheaper than probing it
  was, and it aims the newest, least-exercised code in the tree at real data — which is
  what this pipeline is for. So SCD2 and the wider model set move ahead of provisioning,
  and every GCP phase moves behind them. Phases 3–5 are new; old 3/4 become 6/7 and the
  live phases follow; `phases/03-plan.md` moved to `phases/06-plan.md` unchanged.
  The trade, stated so it is not rediscovered as a surprise: the programme's original
  argument for BigQuery-early was **reach** — "it runs unattended against a real dataset
  and I trust the numbers" — and this delays that, and makes the first live run likelier to
  fail on several fronts at once. Accepted, because the research doc's own sequencing rule
  is "let the real models generate the punch-list", and a wider model set generates more of
  it per live run. What the reorder buys unconditionally: when the live leg finally runs,
  every model and the equivalence invariant over all of them are already green offline, so
  a failure there is a **backend** failure and nothing else.
- 2026-09-08: **what the fixture can actually support**, measured before the phases were
  written rather than assumed: 34 renamed repos, **2 repo names reused across different
  `repo_id`s** (an adversarial case for anything keyed on name), and only **4** renamed
  actors. `silver.actor_naming` is built anyway (human): the value is exercising the
  grammar a second time on a different key and clock, not statistical weight.
- 2026-09-08: **both succession partition postures are reachable from one pipeline**, which
  was not obvious. `incremental_shapes.md` §"The succession grain" admits event-time
  partitioning alongside arrival partitioning, and our source is event-time-partitioned —
  so the deliberate previous-day redelivery lands in a **closed** partition and drives the
  append-only probe's late-arrival classification (a row-count increase in a closed
  partition is a late arrival, never `SourceMutationProfileViolated`). That is the costlier
  and less-exercised of the two postures, and we get it by construction. The arrival
  posture is then one column away: `ingested_date` is the **loader's own stamp**, not
  anything GitHub Archive supplies, so the replay driver and the phase-7 loader can both
  add it without touching `sample.sql`.
- 2026-09-08 (human): **the fan-out earns a `payload` re-pin.** Phase 4 re-pins `sample.sql`
  to project `payload` and regenerates the fixture (roughly double the scan — ~12 GB,
  ~US$0.06 estimated). Narrowing the payload to typed fields inside the sample query was
  rejected: the fan-out's whole point is typed extraction *from JSON*, and a pre-extracted
  column set removes the shape being tested. If 64k raw payloads make the committed fixture
  unreasonably large, phase 4 shortens the day range rather than narrowing the payload —
  fewer rows of the real shape beat more rows of the wrong one.
- 2026-09-08: **`manual.repo_watchlist` loses its motivation.** The research doc wanted it
  because `Technique::ColumnScopedMerge` had no reachable shipped shape; `docs/TODO.md`
  records that gap resolved 2026-08-09 by `20260809-sensitivity-precision.md`, with
  `ValueEnrichedRecipe` staging the shape and a conformance test proving it end to end.
  `gold.events_enriched` is built instead and is the same shape occurring on a real
  pipeline rather than in a testkit recipe — a weaker reason than the original, and an
  honest one.
- 2026-09-08 (plan step): **phase 2's plan was already written and is kept, not rewritten.**
  The plan step of 2026-09-07 wrote `phases/02-plan.md` but never flipped the row, so the
  loop re-selected the phase; the human's reshape then revised the plan in place. It is
  corrected rather than replaced: the phase numbers it cited moved under the reshape (the
  loader is now 7, the first live run 8), `payload` and the fan-out are **deferred to
  phase 4** rather than out of scope — so nothing built here may assume `payload` exists
  or make adding it a breaking change to the source declaration — and the commit message's
  body still described the superseded overlapping-window shape. One task is added:
  `examples/github_activity/README.md` carries the same superseded description and is
  corrected with the models. No phase rows are reshaped; there is no phase-1 summary to
  reshape from, phase 1's findings having landed in this log directly.

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

- 2026-09-08 (phase 2 implement): **`silver.events_deduped`'s lookback is `allow_full_scan:
  true` + declared `key_recurrence`, not a Form-B WHERE filter**, departing from phase 2's
  plan text. A Form-B lookback needs a source column independent of event time that
  correlates with when a row became visible (`events_parsed`'s `arrival_time`);
  `raw.github_events` has none in this phase (the redelivered duplicate is byte-identical,
  including `created_at`), so any self-referential WHERE filter on `created_at` is
  tautological. Verified empirically both ways: removing `allow_full_scan` still produces
  correct dedup counts, because a keyed `MERGE` is idempotent regardless of window width —
  there is no "narrow it and duplicates survive" failure mode to construct here, unlike
  `events_parsed`'s. The negative control that does exist and is tested instead: a duplicate
  pair violating the declared zero-width `key_recurrence` fails transactionally
  (`KeyedRecurrenceBoundViolated`). Mirrors `examples/web_analytics/silver/
  events_deduped.sql`'s own precedent exactly. Full write-up: `phases/02-summary.md`.
- 2026-09-08 (phase 2 implement): `mutation_profile.lateness: '6 hours'` on
  `raw.github_events` is a placeholder, not the measurement task 2 asked for — the committed
  Parquet fixture carries no ingestion-time column to measure real shard lag against
  offline. Harmless: lateness is orchestration-only. Revisit once a session has live
  BigQuery access (phase 7+).

- 2026-09-08 (phase 3 plan): **two physical relations, one per succession posture.** The
  arrival posture needs a loader-stamped `ingested_date`, which the event-time posture must
  not have as its partition column; declaring two sources over the *same* relation was
  rejected because it would key two differently-partitioned fingerprint sidecars onto one
  table — untested, and not the thing this phase exists to test. So
  `raw.github_events_arrival` is its own relation, same columns plus `ingested_date`, and
  the replay driver stamps both. Note for phase 7: the live loader therefore writes two
  tables from one scan.
- 2026-09-08 (phase 3 plan): **the succession history is per-event, not per-rename.**
  "Keep only the rows where the name changed" needs `LAG`, and the classifier admits exactly
  one *row-local* pre-window filter — so `silver.repo_naming` carries one row per
  `(repo_id, created_at)` with the name in force, and `marts.naming_history` derives the
  actual renames downstream. Not a compromise: it is what the grammar's row-locality rule
  forces, and it is worth having written down before someone reads the model and assumes a
  filter was forgotten.
- 2026-09-08 (phase 3 plan): **the fixture already contains a second fold-once population,
  measured before planning.** Beyond the deliberate redelivery there are 139 `(repo_id,
  created_at)` and 145 `(actor_id, created_at)` ties in the sample — real events at the same
  second — and **none** of them disagree on the projected name. So they fold once exactly
  like a redelivery rather than raising `SuccessionClockTie`, and the fold-once leg gets
  free extra coverage. Had any tie disagreed, `repo_naming` would have been unbuildable
  without a clock refinement; it does not, so no clock change is made.
- 2026-09-08 (phase 3 plan): **no delete/tombstone leg in this pipeline.** GitHub's
  `DeleteEvent` is a branch or tag deletion, not a repo or actor deletion, so a
  `QUALIFY NOT <flag>` over it would assert something untrue about the entity's history.
  The tombstone ledger stays covered by `examples/scd2_succession`; this pipeline covers the
  rename stream and the two postures. Criterion 9 asks for neither.
- 2026-09-08 (phase 3 implement): **the succession clock column must be projected
  verbatim, not aliased away** — `crates/smelt-runtime/src/maintenance_driver/succession/
  execute.rs` resolves the clock column's type from the model's own output schema by name,
  so `effective_ts AS valid_from`-style aliasing (which
  `examples/scd2_succession/models/customer_history.sql` itself does) fails at run time
  with "clock column has no resolved output type". `silver.repo_naming`/
  `silver.actor_naming` project `created_at` bare. The pre-existing `scd2_succession`
  example has never actually been executed for real (only via `explain_maintenance`'s
  static plan-report path and the generative `maintenance_conformance` suite, whose
  `SuccessionRecipe::new_lead` projects the clock column unaliased) — worth a follow-up.
- 2026-09-08 (phase 3 implement): **a genuine full-refresh/incremental divergence for
  succession models with ties, found and recorded, not fixed.** `silver.repo_naming`/
  `silver.actor_naming` fail criterion 7's row-count equivalence: the incremental
  window-forward patch loop folds same-`(key, clock)` rows via its `MERGE ... ON`
  addressing; `--full-refresh`'s `emit_succession_full_rebuild` re-runs the model's raw
  compiled `SELECT` with no such addressing and keeps every duplicated row. Measured
  exactly — 139 extra rows (`repo_naming`), 145 (`actor_naming`), matching the fixture's
  own same-second tie counts — and asserted explicitly rather than silently tolerated
  (`crates/smelt-cli/tests/github_activity_replay.rs::full_refresh_matches_incremental_replay`).
  `marts.naming_history` is unaffected (its `LAG` filter drops the duplicate identically
  on both legs). A `SELECT DISTINCT *` fix was tried and found insufficient — `LEAD`/`LAG`
  over an un-deduped source gives tied rows *different* computed values via the window's
  arbitrary tie-break order, so only byte-identical full-row duplicates fold (50 of 139).
  The general fix needs an aggregate fold on `(key_cols, clock_col)` (e.g. `MAX` per other
  column), threading the full output schema into `emit_succession_full_rebuild` and
  touching its statement-parity fixtures and every succession consumer — real,
  well-scoped work for `docs/outcomes/20260906-scd2-keyed-succession`'s decision log, not
  this pipeline. Also: `emit_succession_full_rebuild` never runs the clock-tie probe, so a
  content-*disagreeing* tie would silently corrupt a full-refresh today, undetected by
  this fixture (which measured zero disagreeing ties).
- 2026-09-08 (phase 6 implement): **a second, previously-hidden full-refresh/incremental
  divergence, in `gold.events_enriched`, found but not characterised** — see "## Blocked"
  below for the full writeup. Unlike the succession tie divergence above, this one is
  transient (self-heals within the fixture's replay) rather than persistent, and its root
  cause (why/when the staleness resolves) is unknown. `github_activity_oracle.rs`'s
  comparator, discovery, and registry infrastructure landed and is green; only the
  centrepiece per-window sweep is blocked on this finding.

## Blocked

- 2026-09-09 -- **partially lifted: human action 1 of 2 is done.** A human minted the
  `bigquery-auth.sh` token, so the fixture re-pin below has happened and phase 5 is back in
  flight; the outcome's `**Status:**` is `in progress` again. Human action 2 -- provisioning
  the dogfood GCP project (phase 7 tasks 1-5) -- is untouched and still gates phases 7,
  10-14 and 16. The entry below stands for everything except its phase 5 clause.

- 2026-09-08 -- **the outcome as a whole: every remaining phase needs cloud identity that
  does not exist.** With phases 1-4, 8, 9 and 15 `done`, no workable row is left: 5, 6, 7,
  10-14 and 16 are all `blocked`, and every one of them is gated on a human minting a
  credential. This entry closes the loop's work on the outcome; the wrapper advances to the
  next backlog entry.

  **What the DuckDB half already satisfies:** criterion 4 in all but the typed silver
  fan-out (phases 2-4 -- bronze, dedup, sessionization, both succession models, gold and the
  marts all build with zero diagnostics, wired into per-PR CI), criterion 9 in full (phase 3
  -- both partition postures and the redelivery-folds-once leg), criterion 7's DuckDB half
  (phase 8 -- `every_window_matches_the_full_refresh_oracle` unignored and green over a
  bounded, non-fabrication staleness registry), criterion 3's *artifact* (phase 9 --
  `scripts/bq-dogfood-loader.sh` derives its load SQL from `sample.sql` under a per-PR
  byte-identity gate; only its deployment and cost measurement are outstanding), criterion 8
  in interim form (phase 15 -- `docs/handoffs/2026-09-08-github-activity-findings.md`, the
  DuckDB-half findings the three downstream outcomes consume), and criterion 10 (gates green,
  no ratchet lowered).

  **What is unmet, and why none of it is loop work:** criteria 1, 2, 5 and 6 entirely, plus
  the BigQuery halves of 3, 7 and 8, and the `payload`-dependent tail of 4. Two distinct
  human actions unblock them, in this order:
  1. **Phase 5's fixture re-pin** (independent of the rest): run `bash
     scripts/bigquery-auth.sh` and supply the passphrase for the encrypted service-account
     key, then `bash examples/github_activity/refresh_sample.sh` against a `sample.sql`
     re-pinned to project `payload` (~12 GB scanned, ~US$0.06, already accepted in the
     decision log). Everything after the committed fixture is ordinary loop work.
  2. **Phase 7 tasks 1-5** -- create the dogfood GCP project, link billing, enable
     `bigquery.googleapis.com` and `billingbudgets.googleapis.com`, create the no-expiry
     dataset and the budget alert, create `smelt-dogfood@<PROJECT>.iam.gserviceaccount.com`
     with `bigquery.jobUser` + dataset `WRITER`, grant `roles/iam.serviceAccountTokenCreator`
     on it to the human account, and run the impersonating `gcloud auth
     application-default login --impersonate-service-account=...`. That single act unblocks
     phase 7's own gate and then 10-14 and 16 in order; the loader they deploy is already
     written and tested.

  **How to resume:** flip this file's `**Status:**` back to `in progress` after either human
  action -- the loop will pick the outcome up again from the first `blocked` row that the
  action unblocks (re-mark it `pending`).

- 2026-09-08 -- **phases 10-14 and 16, one shared gate: no cloud identity exists.** Phase 10
  (deploy the loader and measure cost per run), 11 (first live full refresh), 12 (three
  incremental windows), 13 (dual-target parity), 14 (both-target oracle) and 16 (the live half
  of the findings handoff) all require the dogfood GCP project that phase 7 is itself blocked
  on. Re-verified this iteration rather than assumed: `~/google-cloud-sdk/bin/gcloud auth
  list` reports "No credentialed accounts" and
  `~/.config/gcloud/application_default_credentials.json` does not exist, unchanged since
  phases 7, 8 and 9. Nothing in the repo was changed for these phases.

  **What a human must do:** exactly `phases/07-plan.md` tasks 1-5, as spelled out in the phase
  7 entry below -- create the project, link billing, enable `bigquery.googleapis.com` and
  `billingbudgets.googleapis.com`, create the no-expiry dataset and the budget alert, create
  and grant `smelt-dogfood@<PROJECT>.iam.gserviceaccount.com`, then
  `gcloud auth application-default login --impersonate-service-account=...`. Once that exists,
  phase 7's verification gate and then phases 10-14 become executable in order; the loader
  artifact they deploy is already written and tested (`scripts/bq-dogfood-loader.sh`, phase 9).
  Phase 5's separate credential need (a `bigquery-auth.sh` passphrase for the `payload` re-pin)
  is independent of this and still stands.

- 2026-09-08 — **phase 7 (provision the dogfood project), attempted, blocked immediately.**
  Per this file's own header ("phases 7–13 are human-gated ... phase 7 must emit
  `<<PHASE_BLOCKED>>` rather than attempt it") and `phases/07-plan.md`'s task split, tasks
  1–5 (create the GCP project, link billing, enable APIs, create the no-expiry dataset,
  create the budget alert, create the `smelt-dogfood@` service account and grant it, run
  the impersonating ADC login) are human-executed — they spend real money and mint real
  cloud identities, which a headless implement step cannot do. Checked before touching
  anything: `gcloud` is present at `~/google-cloud-sdk/bin/gcloud` (SDK already installed,
  matching D3's premise) but has **no active account** (`gcloud projects list` →
  "You do not currently have an active account selected"), so none of tasks 1–5 have
  happened yet. `.claude/settings.json` in this worktree does **not** currently carry the
  four `Bash(gcloud*)`/`Bash(bq*)` deny entries the plan describes removing — only the
  `scripts/bigquery-*.sh` self-target denials and the `gcloud-smelt-bq` config-dir `Read`
  deny are present — so that removal is either already done elsewhere or was never needed
  in this worktree; either way it is moot until the human side exists to protect.
  Tasks 6/7/9 (mise `setup-gcloud` task, the settings edit, recording the decision log)
  were deliberately **not** done standalone: doing them ahead of an actual provisioned
  project would produce an untestable, unverifiable partial phase (the plan's verification
  gate requires reading back the live dataset's `defaultTableExpirationMs` and confirming
  the impersonated identity is real — neither is checkable without a project). No repo
  changes were made this iteration.

  **What a human needs to do before this phase can proceed:** `phases/07-plan.md` tasks
  1–5 — create the project, link billing, enable `bigquery.googleapis.com` and
  `billingbudgets.googleapis.com`, create the no-expiry dataset, create the budget alert,
  create `smelt-dogfood@<PROJECT>.iam.gserviceaccount.com` with `bigquery.jobUser` +
  dataset `WRITER`, grant `roles/iam.serviceAccountTokenCreator` on it to the human
  account, then run the impersonating `gcloud auth application-default login
  --impersonate-service-account=...` and set the default/quota project. Once that's done,
  re-run this phase — tasks 6/7/9 and the full verification gate become executable in one
  pass.

- 2026-09-08 — **phase 6 (per-window full-refresh oracle), centrepiece test only.**
  **Resolved by phase 8** (see the decision log entry above) — the centrepiece test is now
  unignored and green, on a corrected understanding: the staleness does not self-heal (that
  claim was wrong), the registered bound is non-fabrication rather than convergence, and two
  further divergences this entry's own candidate options did not anticipate were found and
  registered alongside it. Left below for the historical record of how the finding was first
  made.
  `crates/smelt-cli/tests/github_activity_oracle.rs` was built per the phase 6 plan: relation
  discovery (excludes `sources_*`/`_smelt_*`), an `ATTACH`-based `EXCEPT ALL` comparator with
  sample rows, `DIVERGENCE_REGISTRY` with the two known succession entries (`silver_repo_
  naming`, `silver_actor_naming`) bounded by fold-equality on `(key, clock)` rather than a
  magic row count, and 4 of the 5 planned tests are green
  (`oracle_comparison_covers_every_materialised_relation`, `an_unregistered_divergence_fails`,
  `succession_divergence_is_exactly_tied_row_multiplicity`, `registry_entries_are_all_live`).
  The harness extraction into `github_activity_support/mod.rs` (task 1) is done and used by
  both test binaries.

  The centrepiece test, `every_window_matches_the_full_refresh_oracle`, is `#[ignore]`d: at
  the `2026-08-06` checkpoint (2 days replayed) it found a genuine, previously-hidden content
  divergence in `gold_events_enriched` — event id `16854100084` (repo 1183512000, a repo that
  renamed from `laureanoan0/TrainGame` to `laureanomeyer/TrainGame` at `2026-08-06 19:10:54`)
  carries `current_repo_name = laureanoan0/TrainGame` on the incremental leg but
  `laureanomeyer/TrainGame` on the full-refresh oracle, immediately after the rename's window
  finishes. Confirmed by a manual full 30-day rerun that this specific row (and the whole
  relation) is byte-identical between incremental and full-refresh by the *final* window — the
  staleness self-heals within a few subsequent runs, it does not persist. This contradicts, or
  at least sharpens, this repo's own README note ("no maintenance cell is ever derived for
  `gold.repo_dim`'s mutation sensitivity... a repo rename does not re-derive... on already-
  written rows through *any* tracked technique") — some mechanism clearly does eventually fix
  it, and phase 6 does not have the budget to root-cause which one (a redelivery-driven MERGE
  re-touching the same `id`? a periodic full-scan catch-up licensed by `gold.repo_dim`'s
  `allow_full_scan: true`? something else?).

  **Why blocked rather than registered:** the plan's `DIVERGENCE_REGISTRY` shape (a `(key,
  clock)` fold-equality bound) is specific to the succession-model tie/redelivery root cause
  and does not fit this one — the root cause here is dimension-mutation propagation lag, not a
  tie. Inventing a bound (e.g. "resolves within N windows") without root-causing the self-heal
  mechanism risks asserting a made-up number that happens to pass on this fixture while masking
  a real latent bug (or an actual unbounded staleness the 30-day fixture is too short to reveal
  — the oracle only checked days 0-9 and day 29, not every day, so an intermittent recurrence
  in days 10-28 could exist undetected even in the ignored version).

  **Candidate options for the next planner:**
  1. Root-cause the self-heal mechanism (add tracing/logging around `gold.repo_dim`'s and
     `gold.events_enriched`'s per-run maintenance plan; compare the plan issued on the
     `2026-08-06` run against a later run that has already caught up) and, once understood,
     either register a correctly-shaped bound or fix the propagation gap outright.
  2. Extend `every_window_matches_the_full_refresh_oracle` to check *every* day (not just the
     first 10 + final) to rule out a recurring, non-self-healing pattern — more expensive, but
     would settle whether the divergence is actually bounded before spending time on a fix.
  3. Treat this as the criterion-8 `gold.repo_dim` mutation-propagation gap already tracked for
     `docs/outcomes/20260906-bigquery-correctness` (see this file's decision log and
     `examples/github_activity/README.md` "genuine derivation gap" note) surfacing earlier than
     expected, and fold the root-cause work into that outcome instead of this one.

- 2026-09-08 — **phase 5 (`payload` re-pin + typed silver fan-out).**
  **Lifted 2026-09-09** — the token was minted and the fixture regenerated and committed
  (see the decision log's 2026-09-09 entry: population-identical on the nine pre-existing
  columns, 24.3 GB billed). The rest of the phase is ordinary work and is in flight. Left
  below for the record of what the gate was.
  Needs a live
  BigQuery credential: `bash scripts/bigquery-auth.sh`, which prompts a human for the
  passphrase protecting the encrypted service-account key and mints a 1-hour token. What a
  human must do: mint the token, then run `bash examples/github_activity/refresh_sample.sh`
  against a `sample.sql` re-pinned to project `payload` (~12 GB scanned, ~US$0.06 — cost
  already accepted in the decision log of 2026-09-08). If the regenerated Parquet is
  unreasonably large, the decision log's standing instruction is to shorten the day range
  rather than narrow the payload. Once the fixture is committed the rest of the phase is
  ordinary loop work.
