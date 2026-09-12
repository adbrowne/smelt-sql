# Outcome: The GitHub-activity pipeline runs on BigQuery and DuckDB, and the numbers agree

**Created:** 2026-09-06
**Status:** done
**Driver:** split. Phases 2–4, 6, 8 and 9 are loop-grindable (no warehouse, no credentials)
and this outcome sits in `.claude/outcome-backlog` for them. Phase 5 needs a human-minted
BigQuery token for one fixture regeneration (see "## Blocked"); phases 7, 10–14 and 16 are
**human-gated** — they provision cloud resources and run live BigQuery, which a headless
loop cannot do, so those phases must emit `<<PHASE_BLOCKED>>` rather than attempt it.
Phases 15 and 16 bank the evidence and are loop-grindable: phase 16 harvests the *committed
summaries* of the live phases 10–14 and 17, so it needs no credential of its own. Every
phase has now run; nothing is outstanding.
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

1. **Provisioned.** A dataset carrying **no** default table expiration, under a budget
   alert with a documented monthly cap. It lives in the existing `smelt-bq-test-20260816`
   project as a *new* dataset (`smelt_dogfood`, US, cap US$25/month) alongside the
   untouched `smelt_test`: the property this outcome needs is a dataset whose tables do not
   expire, and `defaultTableExpirationMs` is a dataset property, so a second project buys
   nothing the second dataset does not. `docs/research/20260816-bigquery-backend.md`'s
   provisioning decisions are followed except where this outcome's decision log records a
   departure — and the reuse itself is one such departure, with its accepted costs
   enumerated in the 2026-09-09 decision-log entry and in `phases/07-plan.md` §D0.
2. **Reachable, and only this project.** `bq`/`gcloud` are usable from a session against
   the dogfood dataset via ADC impersonating a `smelt-dogfood@` service account, so the
   credential reaches this project and no other — demonstrated, not assumed: a call against
   a different project of the human's is refused. Because `roles/bigquery.jobUser` is
   project-scoped, reachability necessarily extends to `smelt_test` as well; that is an
   accepted cost of criterion 1's reuse, not a scoping failure, and a call against
   `smelt_test` succeeding is the *expected* result rather than a gate failure. The
   blanket `gcloud`/`bq` entries this criterion once required removing from
   `.claude/settings.json` turned out never to be present in this worktree, so the list is
   verified rather than edited. `smelt-bq-test@`'s own credential isolation is untouched
   and is where the real boundary sits: its gpg-encrypted key in a separate
   `CLOUDSDK_CONFIG`, the `scripts/bigquery-*.sh` denials, and
   `Read(//home/andrew/.config/gcloud-smelt-bq/**)` all stay.
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
| 5 | Re-pin `sample.sql` with `payload`, regenerate the fixture, and build the typed silver fan-out (`push_events`, `pr_events`, `issue_events`, `star_events`) | done |
| 6 | Trust the DuckDB numbers: full-refresh oracle vs incremental state across the whole widened model set, banked before any cloud spend | done |
| 7 | Open the dogfood dataset in the existing project: `smelt_dogfood` with no table expiry, a project-scoped AUD 25/month budget, `smelt-dogfood@` service account reached by ADC impersonation, and the `SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS` guard — leaving `smelt_test` and `smelt-bq-test@`'s isolation intact | done |
| 8 | Settle the DuckDB half of criterion 7: characterise and bound `gold.events_enriched`'s per-window enrichment staleness, un-`#[ignore]` `every_window_matches_the_full_refresh_oracle`, and hand the derivation gap to `bigquery-correctness` | done |
| 9 | Author the loader artifact with no cloud: `scripts/bq-dogfood-loader.sh` derives the load SQL *from* `sample.sql` (rolling `_TABLE_SUFFIX` day range, `ingested_date` stamp, deliberate previous-day redelivery slice) plus the `raw.github_events` DDL and the N-day retention bound, gated by a per-PR `--emit-sql` test that proves the projection and filter are byte-identical to `sample.sql` | done |
| 10 | Deploy the loader in the dogfood project and run it: `raw.github_events` created day-partitioned, at least two days loaded, retention verified, cost per run measured and recorded | done |
| 11 | First live BigQuery run: full refresh of the whole model set against the dogfood dataset; record every compile refusal and runtime failure rather than fixing them in place | done |
| 12 | Three or more consecutive incremental windows on BigQuery, run reports captured, frontier and engine-resident state inspected between runs | done (10 of 16 models — see the 2026-09-11 entry) |
| 13 | Dual-target parity: compare every model's output between DuckDB and BigQuery over the same rows; register each difference with a reason or fail | done (14 of 14 relations equal at the final window; one arrival-order divergence registered — see the 2026-09-12 entry) |
| 14 | Trust the numbers on both targets: full-refresh oracle vs incremental state after each window | done (14 of 14 relations byte-equal to their own full refresh at the final window; 8 of 14 at every checkpoint, the other 6 exempt at intermediate ones with a checkable proof — see the 2026-09-12 entry) |
| 15 | Bank the DuckDB-half evidence now: `docs/handoffs/2026-09-08-github-activity-findings.md` carrying the four measured root causes, the five registered divergences and the loader/retention requirements, so the three downstream outcomes' harvest phases can proceed without live BigQuery | done |
| 16 | Extend the handoff with the live-BigQuery findings: every compile refusal, runtime failure and cross-target divergence the live runs surfaced, plus the final punch-list | done |
| 17 | Expand the BigQuery source population to the committed fixture's full thirty days with the real loader, and delete the stray 2026-08-04 slice, so both targets run over the same rows — **runs before 13 and 14** | done |
| 18 | Addendum, recorded after closure: re-run the parity legs on a coarse schedule (six 5-day windows instead of thirty daily ones) and measure what a wider run window buys, whether the two targets still agree, and whether the two schedules converge | done (2.5× faster, 6.3× cheaper; 14 of 14 relations equal across targets at all six checkpoints; the two schedules reach byte-identical state — see the 2026-09-12 addendum entry) |

## Decision log

- 2026-09-12 (phase 18, an addendum after closure): **a coarser run window is a real but
  sub-linear saving, and the two schedules reach identical state.**
  `docs/specs/incremental_shapes.md:555` says the CLI range is a run window rather than a
  per-partition invocation, so phase 13's thirty daily windows were a schedule choice inherited
  from the DuckDB replay driver. Re-running the same thirty days as **six 5-day windows** on
  both targets measures the choice instead of assuming it. **5× fewer runs bought 2.48× the
  model-execution time** (3,086 s → 1,243 s), **2.57× fewer jobs** (2,160 → 842) and **6.30×
  the cost** (US$0.29 → **US$0.046**); leg wall time fell from ≈61 min to 31 m 47 s. Money falls
  faster than time because BigQuery bills a 10 MB per-table minimum many times over; time falls
  slower than the run count because a `PerPartitionOnly` model still executes one partition at
  a time inside a wider window.

  **Which models collapsed, and the uncomfortable part.** Eleven of the fourteen sit at
  4.7–65 s per window with no dependence on window width — the `FullyBatchSafe`/`BoundedSafe`
  shapes the spec says run as a single query for any run window. The two succession cells and
  the cumulative aggregate sit at 120–157 s, roughly 5× the rest and roughly flat, and together
  they are **64% of all per-model execution**. So the three models a wider window does not help
  are the three most expensive ones in this pipeline. The direct per-model fine-vs-coarse ratio
  is measured on DuckDB (4.40× overall; 2.84× for the cumulative model, 4.17–4.18× for the
  succession pair, 4.5–6.5× for everything else) because BigQuery's fine-schedule run manifests
  live under the gitignored `.smelt/` that `clear` removes.

  **The result worth having.** Coarse w06 and phase 13's fine w30, compared in full by the same
  committed comparator with nothing exempt: **zero rows in either direction on all fourteen
  relations**. Two different valid run sequences over identical inputs reach byte-identical
  state — a strictly stronger claim than either sequence alone, and the first direct test of
  the schedule-independence the run-window rule implies. Cross-target parity holds at **all
  six** checkpoints, better than phase 13's natural pair only because a coarse schedule forces
  the preloaded DuckDB leg (the loader is `cadence: 1 day` and gets the window's first day), so
  the `bronze_events` arrival lag cannot arise.

  **What did not converge, and it is not the data.** The interval frontier for
  `silver.repo_naming` and `silver.actor_naming` reads `2026-08-10 → 2026-09-04` on the coarse
  leg where phase 13 recorded `2026-08-05` for every interval-addressed model. It understates
  coverage rather than losing anything. Window width is ruled out by the six models that ran
  the same windows and did record 08-05; what is left is the succession cells'
  `succession_full_rebuild` under this schedule's first-window `--full-refresh`. That is an
  inference from this run's own evidence, not a second measurement, and it goes to
  `20260906-bigquery-correctness` next to punch-list item 1.

  **The outcome stays `done`.** No criterion's verdict moves, no gate was lowered, no registry
  entry was added or removed, and `13-parity.json` / `14-equivalence.json` were not overwritten.
  `scripts/bq-dogfood-parity.sh` was extended rather than forked (`PARITY_WINDOW_DAYS`,
  `PARITY_FIRST_FULL_REFRESH`), both defaulting to the fine schedule. Sources read only,
  65,583 rows each after the run; nothing written outside `smelt_dogfood`; `target: dev` never
  unpinned. Evidence: `phases/18-summary.md`, `18-parity.json`, `18-parity.md`,
  `18-convergence.json`.

- 2026-09-12 (phase 16, and the outcome's close-out): **the evidence is banked, criterion 8
  is met, and the outcome is `done`.** `docs/handoffs/2026-09-08-github-activity-findings.md`
  is no longer interim: its live section banks phases 10–14 and 17 in phase order with each
  phase's own measured cost, a nineteen-row findings table in which every row names the model
  and the statement that provoked it, the backend-agnostic items kept out of the backends'
  punch-lists, an operational recipe for the next live run, and a nine-item final punch-list
  with an owner per item. Nothing in it was re-derived or re-measured here and no cloud call
  was made: every number traces to a committed summary. Four string-level gates hold the
  document — that both halves have landed, that every live-findings row names a provoking
  model and statement (with a scoped, non-vacuous scan behind it), and that the coverage the
  met criteria rest on is stated in the document itself.

  **Criteria met, and on what evidence.** 1 and 2 — phase 7, dataset properties and the
  cross-project job refusal read back from the API. 3 — phase 10, the loader deployed and its
  projection gated byte-identical to `sample.sql`, cost measured per job. 4 — phases 2–5, the
  whole pipeline green on DuckDB per-PR. 5 — phase 12, a full refresh plus three consecutive
  windows, run reports captured. 6 — phase 13, fourteen relations byte-equal between targets
  at the final window, gated over `13-parity.json`. 7 — phase 14 on BigQuery plus
  `every_window_matches_the_full_refresh_oracle` on DuckDB. 8 — this phase. 9 — phase 3, and
  re-exercised live: both succession models compare byte-equal across targets and against
  their own full refresh at the final window. 10 — this phase's own `verify-phase.sh` run.

  **What is not covered, stated rather than rounded up.** The live half is **14 of 16
  models**: `silver.actor_sessions` and `marts.daily_active_contributors` are refused at
  compile time on GoogleSQL over an INTERVAL `RANGE` lookback frame and run on DuckDB only —
  the window-frame lowering seam is punch-list item 3, not done here. The BigQuery
  equivalence check is **7 of 30 windows**, the set phase 13 measured and phase 14 reused; at
  six of those seven, six relations are additionally exempt on the oracle side, each with a
  checkable proof, because a full refresh over a static source is not window-bounded
  (punch-list item 1, the consequential finding). There is **no BigQuery CI tier**, by
  standing decision: the committed reports are what run per-PR, and the live legs are
  re-runnable by hand. And the live evidence has a date — the source tables' oldest partition
  expires 2026-09-19, so a later comparison must re-load the missing days first.

- 2026-09-12 (phase 14, executed live): **the numbers are trustworthy on both targets;
  criterion 7 is met, and criterion 6 stands from phase 13.** On BigQuery, after the fixture's
  full thirty-window incremental run, every one of the **fourteen** compared models' maintained
  state is byte-equal to a full refresh over the whole population — zero rows in both
  directions of a whole-row `EXCEPT ALL`, with an **empty** divergence registry. Eight of the
  fourteen are equal at *every* one of the seven declared checkpoints (1, 2, 3, 5, 10, 20, 30 —
  phase 13's set, reused so the two phases' claims are about the same state). The oracle ran on
  a third committed target, `bigquery_oracle`, writing to `smelt_dogfood_oracle` and reading
  the **same** physical source tables via a `bigquery_oracle:` entry in both sources' `name:`
  maps; without that entry the oracle reads an empty table and the sweep passes vacuously, so
  the gate on it was written RED first and observed failing on exactly that value. The DuckDB
  half was re-run and cited, not rebuilt (`every_window_matches_the_full_refresh_oracle`, 30
  windows, 16 models, 317.6 s). Coverage is stated rather than implied: the BigQuery half is
  **14 of 16 models** (the INTERVAL-`RANGE` compile refusal) at **7 of 30 windows**.

  **The substantive finding, for `20260906-bigquery-correctness` to take a view on: a full
  refresh on BigQuery is not a window-bounded oracle for six of the fourteen models.**
  `--full-refresh --event-time-start X --event-time-end Y` bounds the source scan for eight
  relations and does not for the two succession cells, the dimension declaring
  `allow_full_scan`, `marts.naming_history`, and the two relations enriched from that
  dimension. Because BigQuery's source statically holds all thirty days from before window 1,
  those relations' oracle at an intermediate window refreshes over inputs the incremental leg
  had not yet seen — so the invariant's antecedent ("a full refresh over the inputs seen so
  far") does not hold there and comparing against it would measure arrival order, phase 13's
  controlled variable. Measured, not inferred: four of the six produce output at window 1 that
  is **byte-identical to their output at window 30**, and the other two agree exactly once the
  single column inherited from the dimension (`current_repo_name`) is projected away — row sets
  and every other column match. Each exemption is a checkable proof recorded per row in
  `14-equivalence.json`, applies **only** at intermediate checkpoints, and is ratcheted
  two-sided against the report; a failing proof fails the sweep. Whether the `--event-time-end`
  bound *should* reach those source scans is a product question this phase deliberately does
  not answer. It is invisible on DuckDB, where the oracle stages a truncated source, so it
  could only have surfaced against a warehouse-resident one.

  Consequently **criterion 7 rests on**: the final window, where the inputs seen so far *are*
  the whole source and nothing is exempt, across all fourteen relations on BigQuery; plus all
  thirty windows across all sixteen models on DuckDB. Five per-PR gates read the committed
  `14-equivalence.json`, including one that fails if anything is ever exempt at the final
  window. **Criterion 6 is met** (phase 13) and **criterion 7 is met** with the coverage caveat
  above.

  Two structural consequences worth carrying forward. The comparator, exclusions, divergence
  vocabulary and landing seam moved into `crates/smelt-cli/tests/bq_parity_support/` and are
  shared by the dual-target and equivalence suites — two claims, one primitive, with phase 13's
  sixteen tests passing unchanged through the move. And the scoped dogfood service account
  could **not** create the oracle dataset (`bigquery.datasets.create` denied), which is the
  phase-7 provisioning design working as intended; dataset lifecycle moved to a
  human-credential sibling script (`scripts/bq-dogfood-oracle-dataset.sh`) rather than the SA's
  grant being widened. `smelt_dogfood_oracle` was created without a default table expiration
  and **dropped at the end of the phase, confirmed gone**; `smelt_dogfood` still holds its 20
  tables and both source tables still hold 65,583 rows each, read-only throughout. Cost
  **US$0.06** — a fifth of phase 13's, because the thirty windows of incremental execution were
  reused rather than re-run. Detail in `phases/14-summary.md`.

- 2026-09-12 (phase 13, executed live): **the two targets agree.** The fixture's thirty
  windows ran on both targets over phase 17's one shared population, both legs excluding the
  same compile-refused pair, and at the final window all **fourteen** compared relations are
  byte-equal in both directions of a whole-row `EXCEPT ALL` — zero rows on either side.
  Thirteen of the fourteen agree at *every* compared checkpoint. `silver.events_deduped`
  reaches exactly 64,313 rows, the source's distinct-`id` count, so the loader's deliberate
  1,270-row at-least-once redelivery folds once live on BigQuery across twenty-nine window
  boundaries.

  **The one divergence is about arrival order, not about either engine, and that is measured
  rather than argued.** `bronze_events` — the pipeline's only whole-source rebuild — is short
  the tail the DuckDB leg has not loaded yet at each intermediate checkpoint
  (62,382 → 59,605 → 57,217 → 50,406 → 32,251 → 11,536 → **0**), always with the DuckDB side a
  strict subset. Replaying the DuckDB leg over a source staged up front
  (`run_incremental.py --preload-source`, which leaves `load_day.sh` unchanged and relies on
  its own per-day idempotence) removes the difference at **every** checkpoint including the
  first. So it is registered as one `TARGET_DIVERGENCE_REGISTRY` entry under a new
  `ArrivalLag` bound that still refuses a row lost from inside the lagging leg's own loaded
  range — the clause that stops the entry from licensing a real row-loss bug — and nothing is
  handed to `20260906-bigquery-correctness`.

  **Comparison points were measured before being chosen**, per D2. Exporting
  `github_events` (65,583 rows, the dataset's widest relation) through the real landing path
  took 44.29 s — 1,481 rows/s — which puts a thirty-checkpoint sweep at ≈7.2M exported rows
  ≈81 min, roughly doubling the leg's ~51 min of model execution. Seven checkpoints
  (windows 1, 2, 3, 5, 10, 20, 30 — 1.42M rows, ≈9 min realised) were declared instead. The
  final window is compared in full over all fourteen relations and that is now a gate
  (`the_two_targets_agree_at_the_final_window`), not prose; the six earlier checkpoints earned
  their place by being the only reason the arrival lag is visible at all.

  The liveness ratchet the offline half deliberately left out now exists:
  `registry_entries_are_all_live` checks the registry against the committed
  `13-parity.json` in both directions, and `the_sweep_fails_closed_on_an_empty_registry` was
  rewritten to pass the registry as a parameter so the fail-closed control survives the
  registry becoming non-empty rather than being retired by it.

  Cost **US$0.29** over 2,160 jobs (58.46 GB billed, mostly the 10 MB per-table minimum
  applied many times), inside the US$1 stop gate. Ran 2026-09-11/12 UTC, a week inside the
  2026-09-19 partition-expiry deadline. One operational trap worth carrying: a concurrent
  `cargo test -p smelt-cli` rebuilt `target/debug/smelt` **without** `--features bigquery`
  mid-leg and the next window refused at backend construction — anything driving a live
  warehouse for an hour must run against a pinned binary copy. Detail in `phases/13-summary.md`.

- 2026-09-12 (phase 17, executed live): **the two targets now hold the same population, and
  it is byte-identical.** 28 days (2026-08-07 … 2026-09-03) were loaded with
  `scripts/bq-dogfood-loader.sh --emit-sql`'s own output — phase 10's
  `` `raw. `` → `` `smelt_dogfood. `` substitution and nothing else — in strict calendar
  order, every one of the 56 `INSERT`s returning exactly the fixture's expected row count on
  the first attempt. The 75-row 2026-08-04 residue was then deleted from each table by
  explicit predicate; no table was dropped or truncated. `smelt_dogfood.github_events` now
  holds **65,583 rows over 30 partitions with 64,313 distinct ids**, and those 64,313 rows
  are **byte-identical to the committed fixture** on
  `(id, created_at, type, actor_id, repo_id, repo_name, payload length, payload MD5)` —
  compared in full, not sampled: zero rows differ, zero ids on either side only. All 28
  redelivery slices match the fixture's `MOD(id,50)=0` counts exactly. So `githubarchive`
  has **not** drifted from the fixture, and the fixture's reproducibility claim holds against
  a live re-derivation 26 days later.

  Cost: **90.60 GB billed, US$0.45** at $5/TB (90.50 GB of it the loads; the dry-run
  projection of 90.47 GB was 0.03% low), against the US$25/month cap.

  The **2026-09-19 deadline is now live**: retention was read back from table metadata as
  `expirationMs = 3888000000` (45 days) on both tables, unchanged and unraised, and the
  oldest partition in both is 2026-08-05. Phases 13 and 14 must complete before then.

  One consequence to carry: the source is widened but **no model was run**, so every model
  table in `smelt_dogfood` still holds the old 6,053-row three-day population, as do
  `_smelt_ledger` and `_smelt_observed_delta`. Phase 13 clears and rebuilds them on its own
  terms. Detail in `phases/17-summary.md`.

- 2026-09-12 (orchestrator, at the user's direction): **the parity basis is the committed
  fixture, and BigQuery is expanded up to it rather than DuckDB narrowed down to BigQuery.**
  The two populations were never comparable — `smelt_dogfood.github_events` held 6,053 rows
  over 2026-08-04…06 while `seeds/github_events_sample.parquet` holds 64,313 events over
  2026-08-05…09-03 and contains *no* 2026-08-04 rows at all, so no filter over the fixture
  could reproduce the warehouse population. The first phase-13 plan closed the gap downward,
  by mirroring the warehouse's three days into a scratch DuckDB source. That is now rejected
  in favour of closing it upward: a new **phase 17** loads days 2026-08-07…2026-09-03 with
  `scripts/bq-dogfood-loader.sh`'s own emitted SQL (28 days, ~US$0.55–1.00 against the
  US$25/month cap) and deletes the 75-row 2026-08-04 residue.

  Three reasons, in order of weight. The shared population becomes the **committed, gated**
  one rather than a scratch artifact nothing else checks. The DuckDB leg then needs no new
  machinery at all — `run_incremental.py`'s thirty-window replay and
  `every_window_matches_the_full_refresh_oracle` already run over exactly those rows, which
  collapses the DuckDB half of criterion 7 to "already met" rather than "rebuild it narrower".
  And criterion 3's property — that the source is what the loader produced, not a copy of a
  local file — is preserved and widened from two days to thirty, instead of being quietly
  side-stepped by uploading the Parquet.

  Consequences: phase 13's schedule becomes the fixture's **thirty** windows, not phase 12's
  four; arrival order becomes a variable to control (BigQuery's source is static across the
  run, DuckDB's arrives day by day) with a declared attribution procedure rather than an
  assumption; and the whole comparison carries a hard deadline — both source tables declare
  `partition_expiration_days = 45`, so the fixture's oldest day (2026-08-05) expires on
  **2026-09-19**. Phases 13 and 14 must complete before then or their oldest partitions
  vanish mid-comparison and read as a divergence.

- 2026-09-11 (phase 12's findings fixed and verified live): **the live model set is 14, not 10.**
  `20260906-bigquery-correctness` closed findings 1a, 1b, 2, 3 and 5 and they were proven against
  `smelt_dogfood` the same day (that outcome's 2026-09-11 entries carry the detail and the two
  further defects the verification itself found). What changed for this outcome: the posture
  baseline is planned by BigQuery (3 recorded partitions for `raw.github_events`, not 5,797), the
  `probes: { cadence: off }` workaround is deleted from `examples/github_activity/smelt.yml`, and
  `gold/` and `marts/` materialise on BigQuery for the first time.

  **Consequences for phase 13 (dual-target parity).** It can compare **14** models rather than
  10. The two outside are `silver.actor_sessions` — whose `RANGE BETWEEN INTERVAL` lookback frame
  is refused at compile time on GoogleSQL, by design and with an actionable message, rather than
  failing at the warehouse — and its one downstream `marts.daily_active_contributors`. Making
  those two runnable on BigQuery needs a window-spec lowering seam in the dialect printer, which
  is its own piece of work, not a phase-13 blocker.

  **What phase 13 still has to do itself:** nothing here compares *values* across targets. The
  BigQuery leg holds three days (6,053 rows) against the DuckDB fixture's thirty (64,313), so the
  populations differ by construction and equal counts are not expected — the comparison over the
  same rows is exactly phase 13's job.

- 2026-09-11 (phase 12, executed live): **the pipeline runs incrementally on BigQuery — a
  full refresh and three consecutive windows, ten of sixteen models, for well under a cent.**
  The T5 downgrade holds in practice, not just in the gate: `silver.events_deduped`, the model
  phase 11 died on, succeeded on all four runs, and after W2 it holds **5,990 rows — exactly
  the source's distinct-`id` count**, so the loader's deliberate at-least-once redelivery is
  folded once, live, across window boundaries. W3 was an empty window on purpose (the
  suppressed-no-op-write shape the T5 path sits on) and changed nothing, correctly. The
  `.smelt/` frontier advanced one day per window and merged into a single interval
  (`2026-08-04 → 2026-08-08`); the dataset holds no ledger, sidecar or tombstone table, which
  is the absence `docs/specs/state.md` declares rather than an oversight. Cost: **1.111 GB
  billed across 258 jobs, ≈ US$0.0056**, all of it on the 10 MB minimum-billing floor;
  `githubarchive` was never touched and the loader was not re-run.

  **Criterion 5 is met with two caveats that are the phase's real product findings**, and it
  is stated that way rather than rounded up:

  1. **The append-only posture probe cannot be planned by BigQuery at all**, so probe dispatch
     had to be turned off for *any* run to complete. Two defects compound:
     `emit_append_only_baseline_snapshot`
     (`crates/smelt-logical/src/maintenance/emit/probes.rs:731`) groups the baseline by the
     **raw** partition column rather than the source's declared `granularity: day`, so a
     TIMESTAMP partition column yields one "partition" per second — **5,797** of them for a
     three-day, 6,053-row source (measured from the state file smelt wrote; the arrival twin,
     whose column is already a DATE, gets 2). Then `build_row_set_table`
     (`crates/smelt-core/src/sql/row_set.rs:55-71`) inlines that baseline as a 5,797-branch
     `SELECT … UNION ALL …` chain, because BigQuery has no table-value constructor, and the
     planner refuses a 692,597-character statement: *"Not enough resources for query planning
     - too many subqueries or query is too complex."* No state hygiene avoids it — one model
     establishes the baseline and the next verifies against it **inside the same run**. The
     granularity half is wrong on every backend; DuckDB's `VALUES` just never complains.
  2. **Six of sixteen models never executed**, on two emitted-SQL defects that reach the
     warehouse instead of being refused at compile time: `gold.repo_dim` emits
     `MAX(…) FILTER (WHERE …)`, which GoogleSQL has no clause for, and `silver.actor_sessions`
     emits `RANGE BETWEEN INTERVAL '2 days' PRECEDING`, where GoogleSQL allows only numeric
     offsets (and the same statement carries a dialect-blind `CAST(NULL AS VARCHAR)` waiting
     behind it). `BackendCapabilities` has 23 `supports_*` flags and none covers either
     construct, so `dialect_seam` has nothing to refuse on. Consequence for **phase 13**:
     dual-target parity can compare ten models, not sixteen, until these are fixed.

  Two further findings worth not rediscovering: the precision half of the degradation
  contract is **invisible at run time** — nothing in the console output or the run report says
  an observed delta was skipped, and `smelt explain` still takes no `--target` — and
  **no single committed configuration serves both targets**: `probes: { cadence: off }` is
  required for BigQuery and silently turns the DuckDB negative control
  `recurrence_bound_violation_fails_the_run` green-when-it-should-fail (red-green confirmed
  both ways). The committed `smelt.yml` therefore keeps probes on and carries the two-line
  `probes:` block commented out with the reason inline; that diff is the reproduction recipe.

  Repo change is one file — `examples/github_activity/smelt.yml` gains
  `state: { mode: intervals }`, without which this project writes **no run report at all**
  (`docs/specs/run_state.md` §"Stateless writes nothing") and the phase's central deliverable
  would not exist. Nothing under `crates/` was touched. Gates green on the committed config:
  `github_activity_replay` 21, `github_activity_oracle` 18, `example_diagnostics` 128,
  `github_activity_loader` 11, `example_workspaces github_activity` 1. Full write-up, with
  verbatim errors, per-run billed bytes and `file:line` for every refusal:
  `phases/12-summary.md`.

- 2026-09-11 (orchestrator): **the T5 block is lifted; phases 12-14 return to `pending`.**
  `20260906-bigquery-correctness` phase 11 landed (`6158dc921`): `realisable_state_structures`
  no longer claims BigQuery or Spark realise `ObservedOutputDeltas`/`FingerprintSidecar`, so
  `resolve_availability` now records a downgrade where it previously recorded none, and the
  three T5 write sites route through `maintenance_driver::records_observed_deltas` — derived
  from the availability layer — instead of comparing dialects. Where the structure is
  unrealisable the write proceeds and only the *record* is skipped, so the cost is downstream
  precision, never correctness: exactly the coarser-but-equivalence-preserving downgraded plan
  the 2026-09-10 entry named as the unblock point. Verified rather than taken on the commit
  message: `cargo test -p smelt-runtime --test state_guard_census --test observed_delta` is
  green (14 + 3 tests), and the new structural gate ties every remaining `dialect != DuckDB`
  guard under `maintenance_driver/` to a structure declared unrealisable.

  Phases 12-14 remain **human-gated** — they run live BigQuery and need a minted credential,
  so a headless iteration must still emit `<<PHASE_BLOCKED>>` for them. What changed is that
  the gate is now only the credential, not a capability hole in the product. Nothing is known
  yet about whether the downgraded plan carries the whole model set through
  `silver.events_deduped`; that is phase 12's first question.


- 2026-09-10 (phase 16 plan): **phase 16 is loop work, not human-gated.** The header's
  driver line listed 16 among the phases needing live BigQuery; that was written before any
  live run existed. Phases 10 and 11 executed and their summaries carry the verbatim job
  output, costs, error strings and `file:line` references, so extending the handoff is a
  pure harvest of committed text — the same shape phase 15 already had for the DuckDB half.
  Driver line corrected; no phase rows added, split or reordered. Phases 12-14 stay
  `blocked` on the T5 gap owned by `20260906-bigquery-correctness` (2026-09-10 entry
  below), and phase 16's job is to make that gate legible in the handoff rather than to
  close it.

- 2026-09-10 (post-phase-11, orchestrator): **the T5 block has an owner and an unblock
  point.** `docs/outcomes/20260906-bigquery-correctness` reopened for it (its decision log
  of the same date carries the analysis). The gap is narrower than phase 11 recorded: the
  availability layer *claims* BigQuery realises `StateStructure::ObservedOutputDeltas`
  while every emitter is DuckDB-only, so `resolve_availability` records no downgrade and
  the driver `bail!`s instead. That outcome's **phase 11** reconciles the two layers and
  turns the three `bail!` sites into recorded downgrades — at which point this outcome's
  phases 12-14 become runnable on the coarser (but equivalence-preserving) downgraded
  plan, without waiting for its phases 12-15 to give BigQuery a real ledger substrate.
  Phases 12-14 stay `blocked` until then; the unblock is a code change elsewhere, not a
  provisioning step here.

- 2026-09-10 (phase 11, first live BigQuery run): **the pipeline stops at
  `silver.events_deduped`, on a hard capability gap that is not BigQuery-specific.**
  `bronze.events` and the first write of the succession/dedup models succeeded against
  `smelt_dogfood`; then
  `crates/smelt-runtime/src/maintenance_driver/driver.rs` refuses with
  `Feature not supported by BigQuery: observed-delta recording for a change-suppressed
  keyed fold (T5)`. The check is `backend.dialect() != SqlDialect::DuckDB`, unconditional
  — so **Spark is equally affected**, and the gap is "the ledger substrate is DuckDB-only"
  rather than anything about BigQuery. Because `events_deduped` sits upstream of the whole
  model set, nothing in `gold/` or `marts/` ran at all. Criterion 5 is therefore **not**
  met and cannot be until that gap is closed; phases 12-14 inherit the block.

  Verified rather than taken on report: the `bail!` is where the summary says it is, and
  it is reached whenever a keyed merge suppresses a no-op write on a second-or-later batch.
  The right shape of fix is arguably not a fix at all but a **downgrade path** — smelt
  already has `MaintenanceStateDowngraded` for ledger-requiring techniques, and this refuses
  hard where that mechanism would have degraded gracefully and said so.

  **Phase 10's finding 1 is properly resolved, not papered over.** The `raw.` → real-dataset
  mapping turned out to be spec'd surface already: the target-aware `name:` override
  (`docs/specs/sources.md`), so both source YAMLs now carry a per-target `name:` pointing at
  the loader's physical tables. The manual `sed` of phase 10 is retired. Cost of the whole
  run: **94 MB billed, ~US$0.0005**.

- 2026-09-10 (phase 11): **adding a second target silently re-pointed the entire project,
  and that is filed as a finding rather than absorbed.** `default_target` falls back to the
  **alphabetically-first** target when `target:` is unset, and `bigquery` sorts before
  `dev` — so merely declaring the new target moved every no-`--target` invocation onto a
  dialect with no transactional merge ledger, silently downgrading `ColumnScopedMerge` to
  `PerGroupRecompute` project-wide. Nothing announced it; it surfaced only as three
  unrelated `github_activity_replay` failures. A config edit that changes the execution
  backend for a whole project should not be inferable from sort order. `target: dev` is
  pinned in this example as the local fix, and the general problem is finding 4 for
  `bigquery-correctness`.

  Two test gates legitimately stopped being zero-diagnostics workspaces, because declaring
  a second backend makes `MaintenanceStateDowngraded` fire for real: the downgrade is
  computed against the union of every declared target. Both were changed to assert the
  **exact** expected diagnostic set one-to-one rather than to widen a tolerance — a
  workspace that starts emitting an unlisted diagnostic, or stops emitting a listed one,
  still fails.

- 2026-09-10 (phase 10, executed live): **the loader is deployed and criterion 3 is met;
  the run cost less than a cent and found a schema drift before it spent anything.**
  `smelt_dogfood.github_events` and `…_arrival` exist, day-partitioned on `created_at` and
  `ingested_date` respectively, both with `partition_expiration_days = 45` — read back from
  `tables.get` rather than inferred from the DDL text, and independently re-verified by the
  orchestrator (`expirationMs = 3888000000`, table-level `expirationTime` **unset**, so
  criterion 1's "no default table expiration" survives as a dataset-only property). Two
  days loaded, 6,053 rows, `payload` non-null on every one.

  **The drift, and why it was invisible.** `--emit-ddl` declared 9 columns while
  `--emit-sql` selected 10: phase 5's `payload` re-pin landed after phase 9 wrote the DDL,
  so `INSERT ... SELECT *` would have failed outright. Phase 9's suite proves the *INSERT
  projection* matches `sample.sql` byte-for-byte but never checked the *DDL* against the
  same source — a one-sided gate on a two-sided derivation. Closed by
  `ddl_columns_match_the_sample_projection` (red-green: red against the pre-fix DDL). The
  generalisable shape — several artifacts independently deriving a schema from one source
  of truth with only one of them gated — is a criterion-8 finding.

  **Cost, measured rather than estimated.** Every `githubarchive` scan was dry-run first.
  Two loads billed 3.469 GB and 3.655 GB (~US$0.017–0.018 each at $5/TB); one run per day
  extrapolates to **~US$0.53/month**, or ~US$0.91 using the heaviest day dry-run in the
  pinned range as an upper bound. Against the AUD 25 budget that is noise, and BigQuery's
  1 TiB/month free tier likely makes the realised bill zero at this volume. Note for phase
  12: the two INSERT statements scan the base query independently, so the billed total is
  roughly **double** the per-statement dry-run figure — the dry run is not the bill.

  **One deliberate departure worth knowing.** The two days were loaded newest-first
  (08-06 then 08-05), chosen from dry-run prices because archive volume trends upward
  through the range; 08-07 priced at ~5.23 GB combined and was skipped under the phase's
  own stop-threshold rather than run. A consequence: day-06's redelivery arm landed 2% of
  day-05 *before* day-05's real rows, which is a genuinely different ordering from the
  in-order case the tests exercise. It behaved correctly — 63 duplicate ids, exactly the
  `MOD(id, 50) = 0` slice, and the arithmetic closes (3,201 + 63 = 3,264) — but nothing
  gates two invocations in either order, which is the third criterion-8 finding.

- 2026-09-10 (phase 10, orchestrator follow-up): **a skip-green test deleted rather than
  repaired.** `loader_script_is_shellcheck_clean` linted one script and returned **green**
  when shellcheck was absent — the failure mode that reads exactly like a pass, and the one
  this repo's own fail-loud discipline exists to forbid. It is superseded on every axis by
  `.claude/scripts/shellcheck-gate.sh` (all 62 scripts, fails rather than skips, runs in
  `verify-phase.sh` and the CI Lint job, with shellcheck pinned in `mise.toml`'s `[tools]`
  so it is always present to fail with). The loader suite is 11 tests, not 12.

- 2026-09-10 (phase 7, executed): **provisioned and verified live; two of the plan's own
  premises turned out to be wrong.** `smelt_dogfood` exists in `smelt-bq-test-20260816`
  (US, `defaultTableExpirationMs` **absent**, read back from the API rather than inferred
  from a successful create), `smelt-dogfood@` holds `bigquery.jobUser` at project scope
  plus `WRITER` on that dataset only, and ADC is
  `type: impersonated_service_account` targeting it. Criterion 2 is demonstrated:
  a query job **created** in `smelt-bq-test-20260816`, and **refused** in both other
  projects on the account (`bigquery.jobs.create` denied).

  Two corrections that cost real time and are worth not rediscovering:

  1. **A dataset list is not an access probe.** The first cross-project check used
     `GET projects/<other>/datasets` and got **HTTP 200** from both of the human's other
     projects — which reads as a scoping failure and is not one. The 200 carries an
     *empty* list: the caller may call the endpoint and simply sees nothing. The probe
     that actually answers the question is `POST .../jobs`, since `jobs.create` is the
     permission that spends money and reads data. `scripts/bq-dogfood-provision.sh`'s
     stage 7 now probes job creation, with a positive control in the dogfood project so a
     blanket refusal cannot pass as a clean result.
  2. **There was never a US$5 budget, and the account is AUD.** The US$5 figure came from
     `bigquery-provision.sh`'s `create` invocation, which this project's own memory records
     as never having been run (budgets need ADC; it was left a manual console step). The
     account had exactly one budget — account-wide, **AUD 25**, no project filter. Posting
     a **USD** budget to an AUD billing account fails with a bare `INVALID_ARGUMENT` naming
     no field, which is a thoroughly unpleasant thing to debug. A project-scoped
     **AUD 25/month** budget (`smelt-bq dogfood cap`, thresholds 50/90/100%) now exists.
     The human's instruction was "$25"; AUD is the only currency the account accepts, so
     that is what was created — flagged rather than silently converted.

  Also settled: **D2 — `bq` works here** (BigQuery CLI 2.1.36, no pyOpenSSL failure), but
  the dogfood path still speaks REST over `curl` like every other script in the repo. And
  the budgets API is the one place that cannot: it goes through ADC, which by then is the
  impersonated service account holding no billing role, so that stage runs on the human's
  own access token with an `x-goog-user-project` header.

- 2026-09-10 (phase 7, repo side): **tasks 6, 6a and 7 landed.** `scripts/bq-dogfood-env.sh`
  is the dogfood entry point: it layers on `bigquery-env.sh` and then unsets
  `SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS` and `SMELT_BQ_ACCESS_TOKEN`, and points
  `SMELT_BQ_DATASET` at `smelt_dogfood`. Verified both ways rather than asserted — the
  dogfood path reports `expiry=UNSET dataset=smelt_dogfood token=UNSET`, the test path
  still reports `expiry=7200000 dataset=smelt_test`. The Cloud SDK is pinned as
  `[tasks.setup-gcloud]` plus an `_.path` env entry (a bare `PATH =` key in `mise.toml` is
  **silently ignored** — measured, `gcloud` stayed unresolvable under `mise exec` until it
  was changed to `_.path`), and the resolver prints the not-yet-existent install location
  rather than an empty string, because an empty PATH entry means the current directory.
  Task 7 confirmed rather than edited: `.claude/settings.json`'s `deny` still carries only
  the seven `scripts/bigquery-*.sh` self-target entries and
  `Read(//home/andrew/.config/gcloud-smelt-bq/**)` — no blanket `gcloud`/`bq` deny has
  reappeared, so the test project's credential isolation is intact and untouched.

- 2026-09-09 (human decision): **reuse `smelt-bq-test-20260816` rather than provisioning a
  dedicated dogfood project; criterion 1 amended accordingly.** Criterion 1 originally
  forbade this project by name. That was over-fitted: the property the outcome needs is a
  dataset whose tables do not expire, and `defaultTableExpirationMs` is a *dataset*
  property, so `smelt_test`'s fatal 24h expiry is escaped by adding `smelt_dogfood`
  beside it rather than by adding a project. Accepted costs, each real and each chosen
  rather than overlooked: a shared bill, so phase 10 must measure cost per run from each
  load job's own `totalBytesProcessed` rather than the project total (the more honest
  measurement anyway); no delete-the-project teardown; the budget cap rises from US$5 to
  **US$25/month**, which loosens the guardrail over the test suites sharing the project;
  and session reachability extends to `smelt_test`, because `roles/bigquery.jobUser` is
  project-scoped and cannot be narrowed to one dataset. What reuse does **not** cost is the
  property the credential design was actually built for — the blast radius that mattered
  was ADC carrying Andrew's whole Google Cloud identity, and a `smelt-dogfood@` service
  account scoped to one project still refuses every other one, so criterion 2's
  demonstration is unchanged and `smelt-bq-test@`'s gpg-encrypted key, separate
  `CLOUDSDK_CONFIG` and `Read`-denied config directory are untouched. Phases 10-14 and 16
  are unaffected in substance; phase 7 loses its project-creation half and is rewritten.

- 2026-09-09 (phase 7 plan): **a table-expiration trap is closed explicitly rather than
  left to an undocumented mitigation, and the earlier statement of it was too strong.**
  `scripts/bigquery-env.sh:31` sets `SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS` unconditionally
  (2h default) and `python/smelt/bigquery_adapter.py:152` stamps it onto the dataset it
  creates; a dogfood run must source that script for `PYTHONPATH`. First reading was that
  every dogfood table would therefore expire two hours after being written. Corrected on
  inspection: the adapter calls `create_dataset(..., exists_ok=True)`, which does not
  modify a dataset that already exists, so once `smelt_dogfood` is created with no default
  expiration the env var never reaches it. The trap bites only a dataset the adapter
  creates itself — which is exactly what happens the first time anyone points the pipeline
  at a fresh dataset name. That mitigation is real but load-bearing and undocumented, and
  the failure it guards destroys history a day later while looking like nothing at the
  time, so phase 7 both unsets the variable on the dogfood path and reads the live
  dataset's `defaultTableExpirationMs` back from the API in its gate.

- 2026-09-09 (phase 5, implement): **the fan-out landed clean — zero divergence, no new
  finding.** The four models entered `github_activity_oracle`'s per-window sweep
  automatically (the comparator discovers relations from `information_schema`, so no test
  file listed them) and matched the full-refresh oracle on all 30 windows;
  `DIVERGENCE_REGISTRY` — emptied by `20260906-bigquery-correctness` phases 3-8 — stays
  empty, and `docs/handoffs/2026-09-08-github-activity-findings.md` needs no fifth root
  cause. Criterion 4 is now met in full on DuckDB. Two things worth carrying forward:
  the archive's trimmed payload does **not** match GitHub's published event schema
  (`PushEvent` has no `commits`/`size`, `pull_request` no `title`/`user`/`merged`), so
  field lists were chosen by probing the fixture rather than the docs; and `ACTION` is a
  DuckDB keyword in column-alias position, which is why the three action columns are
  prefixed. Full write-up: `phases/05-summary.md`.

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

- 2026-09-12 — **nothing is blocked; every entry below is history.** The credential was
  minted, the dataset provisioned, and phases 10–14, 17 and 16 all ran. The outcome is
  `done`. The entries below are kept for the record of each gate and how it lifted.

- 2026-09-11 — **LIFTED (phases 12-14).** `20260906-bigquery-correctness` phase 11 landed the
  downgrade path described below as "more likely the right design", so the refusal is gone and
  the three rows are `pending` again. They still need a human-minted BigQuery credential to
  run; see the 2026-09-11 decision-log entry. The entry below is kept for the history of the
  gap and its diagnosis.

- 2026-09-10 — **phases 12, 13 and 14: the T5 capability gap stops the pipeline before
  they have anything to run on.** Phase 11 got `bronze.events` and the first write of the
  silver succession/dedup models onto BigQuery, then hit an unconditional refusal in
  `crates/smelt-runtime/src/maintenance_driver/driver.rs`: `observed-delta recording for a
  change-suppressed keyed fold (T5)` is implemented for DuckDB only
  (`backend.dialect() != SqlDialect::DuckDB` → `bail!`). `silver.events_deduped` is upstream
  of the entire model set, so `gold/` and `marts/` never materialise on BigQuery at all.

  That makes phase 12 (three consecutive incremental windows) impossible — there is no
  completed first run to be incremental *from* — and phases 13 (dual-target parity) and 14
  (both-target oracle) have no BigQuery side to compare. Marking them `blocked` rather
  than leaving them `pending` so the state reflects the gate honestly.

  **What unblocks them:** a BigQuery realisation of the observed-delta bookkeeping, or —
  more likely the right design — an explicit downgrade path parallel to the
  `MaintenanceStateDowngraded` mechanism that already exists for ledger-requiring
  techniques, so the run degrades and says so instead of refusing. That work belongs to
  `docs/outcomes/20260906-bigquery-correctness` (this outcome's "Out of scope" assigns
  fixes there), and it is not BigQuery-specific: the same check refuses on Spark.

  Phase 16 (extend the handoff with live findings) stays `pending` — phase 11 produced
  real live findings and they can be banked without the rest of the run.

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
  2. **Phase 7 tasks 1-5, as rewritten for reuse on 2026-09-09** -- no project creation and
     no billing link, because the project already exists. Enable
     `billingbudgets.googleapis.com` if it is not on, create dataset `smelt_dogfood` (US, no
     `defaultTableExpirationMs`), raise the budget to US$25/month, create
     `smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com` with `bigquery.jobUser`
     + `WRITER` on that one dataset (carrying `smelt-bq-test@`'s existing ACL entries
     forward), grant `roles/iam.serviceAccountTokenCreator` on it to the human account, and
     run the impersonating `gcloud auth application-default login
     --impersonate-service-account=...`. That single act unblocks phase 7's own gate and
     then 10-14 and 16 in order; the loader they deploy is already written and tested.

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
