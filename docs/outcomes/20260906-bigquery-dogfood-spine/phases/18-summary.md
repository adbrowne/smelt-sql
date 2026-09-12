# Phase 18 summary — the coarse schedule, measured

**An addendum recorded after closure, not a reopening.** The outcome's `**Status:**` stays
`done`; nothing here changes a criterion's verdict.

**Executed live** against project `smelt-bq-test-20260816`, dataset `smelt_dogfood`, via ADC
impersonation of `smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`, on
2026-09-12 UTC. Cost **US$0.046**.

**The question.** `docs/specs/incremental_shapes.md:555` states the CLI
`[--event-time-start, --event-time-end)` range is a **run window, not a per-partition
invocation** — "a daily-partitioned 30-day run is one engine query, one partition-aligned
DELETE over the 30 partitions, one INSERT; per-partition equivalence holds regardless of
run-window size". Phase 13 ran the fixture as **thirty daily windows** purely because the
DuckDB replay driver defines that schedule. This re-runs the same thirty days of fixture as
**six 5-day windows** on both targets and measures what that buys.

**Result in one line:** the coarse schedule is **2.5× faster in model execution and 6.3×
cheaper**, it reaches **byte-identical state** to the fine schedule on all fourteen compared
relations, and the two targets agree at **every one of the six checkpoints** — but the saving
is *not* the 5× a naive reading of the spec sentence would predict, because the three models
executed per-partition rather than per-window pay the full five-day price anyway, and they
are the three most expensive models in the pipeline.

---

## 0 — The basis, re-confirmed before anything ran

The 2026-09-19 partition-expiry deadline was not reached: this ran on 2026-09-12, a week
inside it, and the source still holds all thirty days.

```
{"t": "github_events",         "rows_total": 65583, "distinct_ids": 64313,
 "min_day": "2026-08-05", "max_day": "2026-09-03", "days": 30}
{"t": "github_events_arrival", "rows_total": 65583, "distinct_ids": 64313,
 "min_day": "2026-08-05", "max_day": "2026-09-03", "days": 30}
```

Identical to the figures phase 13 compared against. `github_events` and
`github_events_arrival` were **read only** — never dropped, truncated or reloaded — confirmed
again after the phase (**65,583 rows each**, 20 tables in the dataset). Nothing touched
`githubarchive`, nothing was written outside `smelt_dogfood`, and `target: dev` was never
unpinned.

**Clearing the ground**, exactly as phase 13 did — eighteen tables dropped by explicit name
from a list the script printed first, all billing 0 bytes:

```
about to drop 18 table(s) from smelt-bq-test-20260816.smelt_dogfood:
  _smelt_ledger  _smelt_observed_delta  bronze_events  gold_events_enriched
  gold_repo_activity_daily  gold_repo_dim  marts_naming_history  marts_repo_leaderboard
  marts_star_growth  silver_actor_naming  silver_actor_naming__tombstones
  silver_events_deduped  silver_issue_events  silver_pr_events  silver_push_events
  silver_repo_naming  silver_repo_naming__tombstones  silver_star_events
```

## 1 — The schedule, and how it was expressed

Six windows of five days over 2026-08-05 → 2026-09-04, the first a `--full-refresh`, the
other five incremental, on both targets, both with `-e silver.actor_sessions -e
marts.daily_active_contributors` (the pair GoogleSQL refuses at compile time on the INTERVAL
`RANGE` lookback frame). A checkpoint was taken after **every** window, so the coarse
schedule is compared at 6 of 6 rather than at a declared subset.

`scripts/bq-dogfood-parity.sh` was **extended, not forked**: `PARITY_WINDOW_DAYS` sets the
width, `PARITY_FIRST_FULL_REFRESH` makes the first window a full refresh, and
`examples/github_activity/run_incremental.py` gained the matching `--window-days` /
`--first-full-refresh`. Both default to the fine schedule, so every existing invocation is
unchanged.

**One thing a coarse schedule forces, and it is not obvious.** The DuckDB leg's loader is
declared `cadence: 1 day` and receives the window's **first** day as `{run_date}`, so a
five-day window invokes `load_day.sh` once and would load one day of five. The coarse DuckDB
leg therefore has to be the **preloaded** leg (`duck-preloaded`, `--preload-source`), which
stages the whole source before window 1 — which is also, conveniently, the arrival order the
warehouse-resident BigQuery source already has, so phase 13's `bronze_events` arrival-order
divergence cannot arise here at all. This is recorded in the script header and the driver's
docstring rather than left as a trap.

## 2 — Result 1: the speed-up, measured

**Per-window model execution on BigQuery**, from the leg's own transcript:

| window | range | `built 14 model(s) in` |
|---|---|---|
| w01 (`--full-refresh`) | 2026-08-05 .. 2026-08-10 | **124.20 s** |
| w02 | 2026-08-10 .. 2026-08-15 | **227.04 s** |
| w03 | 2026-08-15 .. 2026-08-20 | **216.26 s** |
| w04 | 2026-08-20 .. 2026-08-25 | **240.15 s** |
| w05 | 2026-08-25 .. 2026-08-30 | **206.68 s** |
| w06 | 2026-08-30 .. 2026-09-04 | **228.95 s** |

Every window reported `built 14 model(s)`, exit 0.

**Side by side with phase 13's baseline:**

| | fine (30 × 1 day, phase 13) | coarse (6 × 5 days, this phase) | ratio |
|---|---|---|---|
| runs | 30 | 6 | 5.0× fewer |
| model execution, total | **3,086 s** (51.4 min) | **1,243.3 s** (20.7 min) | **2.48× faster** |
| model execution, per run | min 62.6 / mean 102.9 / max 127.8 s | min 124.2 / mean **207.2** / max 240.2 s | a 5-day window costs **2.01×** a 1-day one |
| leg wall time | ≈61 min (22:09:34Z → 23:23:08Z, incl. one restart, 7 snapshots) | **31 m 47 s** (03:08:15Z → 03:40:02Z, 6 snapshots) | 1.92× |
| jobs | 2,160 | **842** | **2.57× fewer** |
| billed bytes | 58.46 GB | **9.27 GB** | **6.30× less** |
| cost | US$0.29 | **US$0.046** | **6.30× less** |

The coarse leg's wall time splits as 1,243 s of model execution and ~664 s (11.1 min) of
relation export across the six snapshots (84 exports, 403,828 rows in the final one — the same
figure phase 13 measured at w30).

**The honest reading.** A 5× reduction in run count buys **2.5×** in execution time and
**6.3×** in money. Money falls faster than time because BigQuery bills the 10 MB per-table
minimum many times over and the coarse schedule issues 2.6× fewer jobs against inputs that are
only 65,583 rows; time falls slower than the run count because a five-day window is *not* five
times cheaper than a one-day window for every model — §3.

**No batch-safety warning fired.** `incremental_shapes.md:585` warns when a `FullyBatchSafe`
batch spans more than 30 partition periods; a 5-day window is well inside it, and a scan of
the whole leg transcript for any warning other than the pyarrow `BigQuery Storage module not
found` `UserWarning` returns nothing.

## 3 — Result 1, per model: which models collapsed and which did not

**Per-model, per-window duration on BigQuery** (seconds, from the run manifests in
`.smelt/targets/bigquery/runs/`; w01 is the `--full-refresh`):

| model | strategy (w02–w06) | batch safety | w01 | w02 | w03 | w04 | w05 | w06 | total |
|---|---|---|---|---|---|---|---|---|---|
| `silver.repo_naming` | `succession_patch` | succession | 36.6 | 149.4 | 145.5 | 137.8 | 128.8 | 150.1 | **748.2** |
| `silver.actor_naming` | `succession_patch` | succession | 18.1 | 141.7 | 139.1 | 144.2 | 121.2 | 156.6 | **720.9** |
| `silver.events_deduped` | `cumulative_aggregate` | cumulative | 74.0 | 134.9 | 127.1 | 121.4 | 113.7 | 142.6 | **713.7** |
| `gold.repo_activity_daily` | `deleteinsert` | incremental | 18.1 | 20.6 | 48.9 | 65.4 | 11.2 | 48.8 | 213.0 |
| `silver.pr_events` | `deleteinsert` | incremental | 16.4 | 51.7 | 38.9 | 52.3 | 49.0 | 10.8 | 219.1 |
| `silver.star_events` | `deleteinsert` | incremental | 20.5 | 41.4 | 22.0 | 35.0 | 20.2 | 27.7 | 166.8 |
| `silver.issue_events` | `deleteinsert` | incremental | 30.9 | 28.8 | 12.6 | 10.8 | 39.8 | 37.8 | 160.7 |
| `silver.push_events` | `deleteinsert` | incremental | 19.5 | 12.3 | 30.2 | 25.8 | 29.8 | 18.7 | 136.4 |
| `gold.events_enriched` | `column_scoped_merge` | incremental | 10.8 | 17.4 | 15.3 | 24.0 | 22.5 | 14.6 | 104.6 |
| `bronze.events` | `full_refresh` | — | 9.8 | 8.8 | 15.1 | 11.6 | 12.1 | 7.3 | 64.6 |
| `marts.naming_history` | `full_refresh` | — | 7.5 | 27.4 | 6.8 | 5.0 | 5.8 | 5.3 | 57.8 |
| `gold.repo_dim` | `full_refresh` | — | 7.9 | 6.4 | 11.2 | 5.0 | 5.2 | 4.7 | 40.4 |
| `marts.star_growth` | `full_refresh` | — | 6.8 | 6.8 | 4.9 | 5.2 | 5.1 | 7.5 | 36.4 |
| `marts.repo_leaderboard` | `full_refresh` | — | 7.1 | 6.3 | 4.7 | 5.3 | 4.7 | 5.0 | 33.1 |

(Per-model durations sum to 3,415.7 s against 1,243.3 s of wall, because smelt runs the DAG's
independent models concurrently.)

**The split is stark and it lines up exactly with the spec's execution column.** Eleven of the
fourteen sit between 4.7 s and 65 s per window with no trend in the window's width — those are
the `FullyBatchSafe`/`BoundedSafe` shapes the spec says execute as a **single query for any run
window** (a `BoundedSafe(n)` chunk is 3n clamped to 7–90 partitions, so a 5-day window is one
chunk). Three — the two succession cells and the cumulative aggregate — sit at **120–157 s per
window**, roughly 5× the others and roughly flat, which is what **one partition at a time,
sequential** costs when the window holds five partitions. Those three alone are 2,183 s of the
3,416 s: **64% of all per-model execution in the coarse leg**.

That is why the leg is 2.5× rather than 5× faster. **The three most expensive models in this
pipeline are precisely the three that a wider run window does not help.**

**The per-model collapse ratio, measured rather than inferred.** BigQuery's *fine*-schedule
run manifests are not recoverable — they live under the gitignored `.smelt/targets/bigquery/`,
which `clear` removes by design — so the direct fine-vs-coarse per-model ratio is taken on
DuckDB, over the identical model set, identical windows and identical exclusions (phase 13's
thirty preloaded windows against this phase's six), total `duration_ms` per model:

| model | batch safety | fine (30 windows) | coarse (6 windows) | ratio |
|---|---|---|---|---|
| `marts.star_growth` | — | 343 | 53 | **6.47×** |
| `bronze.events` | — | 35,835 | 7,003 | 5.12× |
| `marts.repo_leaderboard` | — | 4,463 | 889 | 5.02× |
| `gold.repo_dim` | — | 16,486 | 3,351 | 4.92× |
| `marts.naming_history` | — | 16,469 | 3,358 | 4.90× |
| `silver.issue_events` | incremental | 17,087 | 3,525 | 4.85× |
| `silver.star_events` | incremental | 17,043 | 3,517 | 4.85× |
| `silver.pr_events` | incremental | 17,095 | 3,531 | 4.84× |
| `silver.push_events` | incremental | 17,029 | 3,539 | 4.81× |
| `gold.repo_activity_daily` | incremental | 17,023 | 3,544 | 4.80× |
| `gold.events_enriched` | incremental | 5,726 | 1,274 | 4.49× |
| `silver.actor_naming` | succession | 35,903 | 8,583 | **4.18×** |
| `silver.repo_naming` | succession | 35,783 | 8,586 | **4.17×** |
| `silver.events_deduped` | cumulative | 23,357 | 8,233 | **2.84×** |
| **total** | | **259,642** | **58,986** | **4.40×** |

Same ordering, milder gradient: on DuckDB the per-partition penalty is a few milliseconds of
query setup, so `silver.events_deduped` still collapses 2.84×; on BigQuery each extra
sequential partition is another job against a ~5 s floor, so the same model barely collapses
at all. **The cost of not collapsing is a property of the backend, not of the model.**

**One incidental measurement worth keeping.** At w01 the succession models ran
`succession_full_rebuild` under `--full-refresh` and took **18.1 s** and **36.6 s** — rebuilding
the *entire* 64,000-row succession history from the whole source is four to eight times
*cheaper* than patching five days into it incrementally (140–150 s). For a pipeline this size
the incremental path for a succession cell is a pessimisation on BigQuery.

## 4 — Result 2: cross-target parity on the coarse schedule

Same comparator, same rule: whole-row multiset difference (`EXCEPT ALL` in both directions over
`SELECT *`) run inside DuckDB with the BigQuery side landed under the DuckDB leg's own declared
types, relation discovery generic on both sides, final window compared in full. Fourteen
relations on each side at every checkpoint; `=` is zero in both directions.

| relation | w01 (08-05) | w02 (08-10) | w03 (08-15) | w04 (08-20) | w05 (08-25) | **w06 (08-30)** |
|---|---|---|---|---|---|---|
| `bronze_events` | = | = | = | = | = | **=** |
| `gold_events_enriched` | = | = | = | = | = | **=** |
| `gold_repo_activity_daily` | = | = | = | = | = | **=** |
| `gold_repo_dim` | = | = | = | = | = | **=** |
| `marts_naming_history` | = | = | = | = | = | **=** |
| `marts_repo_leaderboard` | = | = | = | = | = | **=** |
| `marts_star_growth` | = | = | = | = | = | **=** |
| `silver_actor_naming` | = | = | = | = | = | **=** |
| `silver_events_deduped` | = | = | = | = | = | **=** |
| `silver_issue_events` | = | = | = | = | = | **=** |
| `silver_pr_events` | = | = | = | = | = | **=** |
| `silver_push_events` | = | = | = | = | = | **=** |
| `silver_repo_naming` | = | = | = | = | = | **=** |
| `silver_star_events` | = | = | = | = | = | **=** |

**Fourteen of fourteen at all six checkpoints, including the first** — better than phase 13's
natural pair, and for a reason that is not a strengthening of the result: the coarse DuckDB leg
must be the preloaded one (§1), so the arrival-order lag that made `bronze_events` diverge at
phase 13's intermediate checkpoints cannot occur. This matches phase 13's *attribution* run,
not its natural one. Row counts and the machine-readable report: `18-parity.md`,
`18-parity.json`.

The w06 row counts are identical to phase 13's w30 in every cell — 65,583 / 64,313 / 9,997 /
5,016 / 42 / 5,016 / 22 / 64,168 / 64,313 / 128 / 366 / 60,643 / 64,174 / 47 — which is §5's
claim previewed by row count before it is made by value.

**The `--full-refresh` at w01 is visible in the intermediate row counts, and both targets do it
identically.** `gold_repo_dim` reads 5,016 at w01 and never moves; `silver_repo_naming` reads
64,174; `marts_naming_history` reads 42. That is phase 14's finding (a full refresh over a
static source is not window-bounded) showing up in the *incremental* leg because this
schedule's first window is a full refresh. It is not a divergence — the two engines produce the
same over-wide result — but it does mean this leg's intermediate checkpoints are a weaker
statement about window-limited maintenance than phase 13's were.

## 5 — Result 3: the two schedules converge

The prize. Coarse w06 against the fine schedule's w30, both on BigQuery, both landed under one
set of declared types and differenced by the **same** comparator the other two claims use
(`crates/smelt-cli/tests/bq_parity_support/`, driven through
`github_activity_bq_oracle::bigquery_incremental_matches_its_oracle_at_every_window` with its
two sides relabelled — see `18-convergence.json`'s `comparator` field). Because the single
checkpoint *is* the final window, nothing is exempt: all fourteen relations are compared in
full.

```
14 relation(s); 0 divergent
test bigquery_incremental_matches_its_oracle_at_every_window ... ok
```

**Zero rows in either direction on all fourteen relations.** Two different valid run sequences
over identical inputs — thirty daily windows run on 2026-09-11, six five-day windows run on
2026-09-12, over a source neither of them modified — reach **byte-identical** state.

That is a strictly stronger claim than either sequence alone. Phase 13 showed one schedule
agrees across two engines; phase 14 showed one schedule agrees with its own full refresh. This
shows the state is a function of the *inputs*, not of the *schedule* — which is what
"per-partition equivalence holds regardless of run-window size" actually asserts, and it had
not been tested.

## 6 — What did *not* converge: the interval frontier for the two succession models

Recorded because it is the one difference the run surfaced, and it is real.

Coarse leg, `.smelt/targets/bigquery/intervals.json`:

```
gold.events_enriched      [2026-08-05 .. 2026-09-04]   cell_frontiers= {}
gold.repo_activity_daily  [2026-08-05 .. 2026-09-04]   cell_frontiers= {}
silver.issue_events       [2026-08-05 .. 2026-09-04]   cell_frontiers= {}
silver.pr_events          [2026-08-05 .. 2026-09-04]   cell_frontiers= {}
silver.push_events        [2026-08-05 .. 2026-09-04]   cell_frontiers= {}
silver.star_events        [2026-08-05 .. 2026-09-04]   cell_frontiers= {}
silver.actor_naming       [2026-08-10 .. 2026-09-04]   cell_frontiers= {}
silver.repo_naming        [2026-08-10 .. 2026-09-04]   cell_frontiers= {}
```

Phase 13's fine leg recorded `2026-08-05 → 2026-09-04` for **every** interval-addressed model.
Here the two succession models are missing the first window, and every other model covers it.
The frontier *understates* coverage — the data for 08-05..08-10 is present and byte-equal, as
§4 and §5 both show — so the consequence is redundant work if that range were re-requested, not
loss.

**Attribution, with its evidence and its limit.** Window width is ruled out by the six models
that ran the same five-day windows and did record 08-05. What is left is that the succession
cells' `succession_full_rebuild` path under `--full-refresh` does not record the requested
interval the way `succession_patch` does. That is an **inference from this run's own evidence,
not a second measurement**: confirming it needs a coarse leg with no first-window full refresh,
which was not run. Owner: `20260906-bigquery-correctness`, alongside punch-list item 1 (whether
`--event-time-end` should bound a full refresh at all), which it plainly neighbours.

## 7 — Cost (stop gate: US$1)

Read from BigQuery job history (`statistics.query.totalBytesBilled`) under the human
credential — the scoped dogfood SA has `roles/bigquery.jobUser` and cannot list jobs
(`Access Denied: … does not have the required permissions ('bigquery.jobs.list')`, reproduced
this session). Everything this addendum issued, from 03:00Z:

```
{"jobs": 842, "totalBytesBilled": 9273606144, "GB": 9.2736,
 "usd_at_5_per_TB": 0.04637, "states": {"DONE": 842}}
```

**US$0.046**, an order of magnitude inside the stop-and-report threshold. That covers the six
windows, the 84 relation exports across six snapshots, the 18 drops (0 bytes each) and the
basis and end-state reads. It brings the live programme's running total from US$0.84 to
**≈US$0.89**.

## 8 — Gates

```
SMELT_BQ_DOGFOOD_LIVE=1 PARITY_MANIFEST=target/phase18/parity-manifest.json \
PARITY_REPORT_OUT=.../18-parity.json \
cargo test -p smelt-cli --features duckdb --test github_activity_dual_target \
  duckdb_and_bigquery_agree_on_every_model
  -> test result: ok. 1 passed; 0 failed        (the coarse cross-target sweep)

SMELT_BQ_DOGFOOD_LIVE=1 EQUIVALENCE_MANIFEST=target/phase18/convergence-manifest.json \
EQUIVALENCE_REPORT_OUT=target/phase18/convergence-raw.json \
cargo test -p smelt-cli --features duckdb --test github_activity_bq_oracle \
  bigquery_incremental_matches_its_oracle_at_every_window
  -> test result: ok. 1 passed; 0 failed        (the schedule-convergence comparison)
```

Both consume the committed comparator; neither introduces a new definition of "equal". The
phase-13 and phase-14 gates over `13-parity.json` / `14-equivalence.json` are untouched — those
artifacts were not overwritten, and no registry entry was added or removed.

`bash .claude/scripts/verify-phase.sh` output is quoted in the commit for this phase.

## 9 — Two hazards this run added to the list

1. **Do not edit a bash script while it is executing.** `scripts/bq-dogfood-parity.sh` was
   patched (an unrelated `stage_report` change) while the leg was mid-flight; bash reads a
   script incrementally by byte offset, so the shifted file produced
   `scripts/bq-dogfood-parity.sh: line 459: syntax error near unexpected token 'in'` **after**
   all six windows and all six snapshots had completed. No work was lost and no partial state
   was written — the error is in the trailing `case` dispatch, past everything — but it could
   as easily have landed mid-loop. `bash -n` on the file passes; the corruption was purely in
   the running interpreter's view.
2. **`SMELT_BIN` must point at a file named `smelt`.** The stage prepends `dirname $SMELT_BIN`
   to `PATH` for the DuckDB leg, so a pinned copy called `smelt-bq` gives
   `FileNotFoundError: 'smelt'`. Pin as `<dir>/smelt`.

## Findings

1. **A coarser run window is a real but sub-linear saving: 5× fewer runs buys 2.5× the
   execution time and 6.3× the cost.** The spec's sentence is correct about what a window *is*;
   it is silent about the fact that a `PerPartitionOnly` model still executes one partition at
   a time inside it, and in this pipeline those models are 64% of the bill.
2. **Two different valid run sequences over identical inputs reach byte-identical state**, on
   all fourteen compared relations on a real warehouse. This is the first direct test of the
   schedule-independence the run-window rule implies, and it passed with nothing exempt.
3. **The three models a wider window does not help are the three most expensive ones.** Anyone
   tuning a schedule should read the batch-safety class first: the saving is concentrated
   entirely in `FullyBatchSafe`/`BoundedSafe` models, and `smelt explain` reports no
   batch-safety class for 8 of 16 models today ([#205](https://github.com/adbrowne/smelt-sql/issues/205)),
   so that reading is not yet possible from the CLI.
4. **A succession cell's full rebuild is cheaper than its incremental patch on BigQuery at this
   scale** — 18–37 s against 140–150 s. Fixture-sized, and it will invert at some volume, but it
   says the incremental path is not unconditionally the cheap one.
5. **The interval frontier is not schedule-invariant for the two succession models** (§6), while
   the data is. Attributed by elimination to `succession_full_rebuild` under `--full-refresh`,
   not to window width.

## Not done / blocked

- Nothing was blocked.
- **The per-model fine-vs-coarse ratio was measured on DuckDB, not BigQuery.** BigQuery's
  fine-schedule run manifests live under the gitignored `.smelt/targets/bigquery/`, which
  `clear` removes, and phase 13's were gone before this phase started. Recovering them means
  re-running the thirty-window leg (≈61 min, ≈US$0.29); it was not judged worth it, and the
  BigQuery half of the per-model question is answered instead by the coarse leg's own
  per-window flatness plus the spec's execution rule (§3).
- **§6's attribution is an inference, not a measurement.** A coarse leg without the
  first-window `--full-refresh` would settle it, and was not run.
- No fix was made to anything, and nothing here reopens the outcome.
