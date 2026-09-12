# Phase 13 summary — the two targets agree

**Executed live** against project `smelt-bq-test-20260816`, dataset `smelt_dogfood`, via ADC
impersonation of `smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`, on
2026-09-11/12 UTC. Cost **US$0.29**.

**Result in one line:** the fixture's thirty windows ran on both targets over phase 17's one
shared population, and at the final window **all fourteen compared relations are byte-equal
in both directions of a whole-row multiset difference — zero rows on either side**. Exactly
one relation, `bronze_events`, differs at the intermediate checkpoints; it is root-caused to
**arrival order**, not to either engine, and the attribution run the plan's D2 prescribes
removes it completely — with the DuckDB leg replayed over a source staged up front, the two
targets agree at *every* checkpoint including the first.

---

## 1 — The basis, re-confirmed before comparing (task 3)

Phase 17's population was re-verified cheaply rather than assumed. Read back this session:

```
{"t": "github_events",         "rows_total": 65583, "distinct_ids": 64313,
 "min_day": "2026-08-05", "max_day": "2026-09-03", "days": 30}
{"t": "github_events_arrival", "rows_total": 65583, "distinct_ids": 64313,
 "min_day": "2026-08-05", "max_day": "2026-09-03", "days": 30}
```

and the 30 per-day counts diffed against the committed fixture with the loader's own
redelivery expectation (`bq_rows(D) = fixture_rows(D) + fixture MOD(id,50)=0 slice of D`,
no redelivery after the last day):

```
{"days_fixture": 30, "days_bq": 30, "mismatching_days": 0,
 "fixture_total": 64313, "bq_total": 65583, "bq_distinct_total": 64313}
BASIS OK
```

The comparison therefore has the basis D1 requires. The **2026-09-19** partition-expiry
deadline was not reached: this ran on 2026-09-11/12, a week inside it.

`github_events` and `github_events_arrival` were **read only** — never dropped, truncated or
reloaded — and no statement in this phase touched `githubarchive`.

## 2 — Clearing the ground (D2)

Eighteen tables were dropped, by explicit name, from a list the script printed first:

```
about to drop 18 table(s) from smelt-bq-test-20260816.smelt_dogfood:
  _smelt_ledger  _smelt_observed_delta  bronze_events  gold_events_enriched
  gold_repo_activity_daily  gold_repo_dim  marts_naming_history  marts_repo_leaderboard
  marts_star_growth  silver_actor_naming  silver_actor_naming__tombstones
  silver_events_deduped  silver_issue_events  silver_pr_events  silver_push_events
  silver_repo_naming  silver_repo_naming__tombstones  silver_star_events
```

all eighteen billed 0 bytes. `examples/github_activity/.smelt/targets/bigquery/` was removed.
The drop list is *discovered* (`INFORMATION_SCHEMA.TABLES` minus the two source tables) and
then **screened**: the script refuses outright if the list ever contains `github_events`,
`github_events_arrival`, or a qualified name. There is no wildcard drop anywhere in it.

## 3 — Comparison points: measured, then declared (D2)

**The measurement.** Before choosing, `smelt_dogfood.github_events` was exported once through
the real landing path (`scripts/bq_dogfood_export.py`, paging `jobs.getQueryResults` over
REST). It is the widest relation in the dataset — it carries `payload`, as do
`bronze_events`, `silver_events_deduped` and `gold_events_enriched` — and at 65,583 rows is at
least as large as any model table the sweep exports, so its rate is a conservative bound:

```
65583 rows -> measure.ndjson
bytes billed: 22384751
rows=65583 bytes=33438241 seconds=44.29 rows_per_sec=1481 MB_per_sec=0.72
```

**What that implies for the two options.** The BigQuery side's compared relations total
403,828 rows at the final window and grow roughly with the window, so a thirty-checkpoint
sweep exports ≈ **7.2M rows ≈ 81 minutes** at the measured 1,481 rows/s — against ~51 minutes
of actual model execution, so it would have roughly doubled the leg's wall time.

**Declared set: windows 1, 2, 3, 5, 10, 20 and 30** — the plan's starting shape, which the
measurement supports. That is **1,421,880 exported rows**, projected ≈16 minutes. Realised
export was faster than the conservative bound, because the model tables are narrower than the
source: the final snapshot moved 403,828 rows in ~135 s (**≈2,990 rows/s**), and the seven
snapshots together cost ≈9 minutes of the leg's wall time.

The final window is compared **in full over all 14 relations**, and that is gated rather than
asserted: `the_two_targets_agree_at_the_final_window` reads the committed report and requires
a zero difference on every relation there. The six earlier checkpoints exist to catch a
divergence that appears and then heals — and they earned their place: they are the only
reason the `bronze_events` arrival lag is visible at all, since an end-state-only comparison
would have shown nothing.

## 4 — The schedule, and what it cost in wall time (task 4)

One window per fixture day, 2026-08-05 … 2026-09-03, on both targets, both with
`-e silver.actor_sessions -e marts.daily_active_contributors`.

| leg | invocation | windows | wall time |
|---|---|---|---|
| DuckDB (natural) | `run_incremental.py --days 30` | 30 | **68.4 s** |
| BigQuery | `smelt run --target bigquery` per window | 30 | **≈61 min** (22:09:34Z → 23:23:08Z, including a restart, see below) |
| DuckDB (attribution) | `run_incremental.py --days 30 --preload-source` | 30 | **78.3 s** |

Every one of the thirty BigQuery runs reported `built 14 model(s)`, exit 0. Per-run model
execution: **min 62.6 s, mean 102.9 s, max 127.8 s, total 3,086 s (51.4 min)**. The last
window's run report, verbatim in the parts that matter:

```json
{ "completed_at": "2026-09-11T23:20:43.485801440Z", "duration_ms": 115830,
  "outcome_counts": { "success": 14, "failed": 0, "skipped": 0 }, "failures": [],
  "external_steps": { "sources.raw.github_loader": {
    "command": ["bash", "load_day.sh", "--date", "2026-09-03"],
    "duration_ms": 14, "outcome": "success" } } }
```

The interval frontier after the leg is `2026-08-05 → 2026-09-04` for every
interval-addressed model, one interval rather than thirty fragments, `cell_frontiers: {}`
throughout.

**One interruption, and its cause, since it is a trap worth writing down.** The leg died at
window 3 with `Error: BigQuery backend not available. Rebuild with --features bigquery`. The
cause was not the cloud: a `cargo test -p smelt-cli` issued from this session while the leg
ran rebuilt `target/debug/smelt` **without** the `bigquery` feature, under the running
script's feet. The fix was to pin a dedicated binary (`SMELT_BIN`) and resume from window 3
(`PARITY_RESUME_FROM`, which keeps window numbers absolute so a checkpoint keeps its label).
The refusal happens at backend construction, before any statement is issued, so no partial
window was written — windows 1 and 2 stood and window 3 re-ran clean.

## 5 — The parity result (D3)

Whole-row multiset difference — `EXCEPT ALL` in both directions over `SELECT *` — run inside
DuckDB with the BigQuery side landed locally under the **DuckDB leg's own declared types**.
Relation discovery is generic on both sides; a relation on only one target is a coverage
failure, and none occurred: **14 relations on each side at every checkpoint**.

`-d/+b` is `duck_only`/`bq_only`; `=` is zero in both directions.

| relation | w01 (08-05) | w02 (08-06) | w03 (08-07) | w05 (08-09) | w10 (08-14) | w20 (08-24) | **w30 (09-03)** |
|---|---|---|---|---|---|---|---|
| `bronze_events` | -0/+62382 | -0/+59605 | -0/+57217 | -0/+50406 | -0/+32251 | -0/+11536 | **=** |
| `gold_events_enriched` | = | = | = | = | = | = | **=** |
| `gold_repo_activity_daily` | = | = | = | = | = | = | **=** |
| `gold_repo_dim` | = | = | = | = | = | = | **=** |
| `marts_naming_history` | = | = | = | = | = | = | **=** |
| `marts_repo_leaderboard` | = | = | = | = | = | = | **=** |
| `marts_star_growth` | = | = | = | = | = | = | **=** |
| `silver_actor_naming` | = | = | = | = | = | = | **=** |
| `silver_events_deduped` | = | = | = | = | = | = | **=** |
| `silver_issue_events` | = | = | = | = | = | = | **=** |
| `silver_pr_events` | = | = | = | = | = | = | **=** |
| `silver_push_events` | = | = | = | = | = | = | **=** |
| `silver_repo_naming` | = | = | = | = | = | = | **=** |
| `silver_star_events` | = | = | = | = | = | = | **=** |

Row counts behind those cells (`duck/bq` where they differ, one number where they are equal):

| relation | w01 | w02 | w03 | w05 | w10 | w20 | w30 |
|---|---|---|---|---|---|---|---|
| `bronze_events` | 3201/65583 | 5978/65583 | 8366/65583 | 15177/65583 | 33332/65583 | 54047/65583 | 65583 |
| `gold_events_enriched` | 3201 | 5915 | 8249 | 14932 | 32758 | 53018 | 64313 |
| `gold_repo_activity_daily` | 399 | 762 | 1165 | 2286 | 5530 | 8529 | 9997 |
| `gold_repo_dim` | 399 | 686 | 956 | 1644 | 3186 | 4438 | 5016 |
| `marts_naming_history` | 1 | 2 | 3 | 10 | 26 | 35 | 42 |
| `marts_repo_leaderboard` | 399 | 686 | 956 | 1644 | 3186 | 4438 | 5016 |
| `marts_star_growth` | 1 | 2 | 3 | 5 | 7 | 12 | 22 |
| `silver_actor_naming` | 3143 | 5856 | 8178 | 14853 | 32647 | 52893 | 64168 |
| `silver_events_deduped` | 3201 | 5915 | 8249 | 14932 | 32758 | 53018 | 64313 |
| `silver_issue_events` | 1 | 1 | 13 | 20 | 47 | 73 | 128 |
| `silver_pr_events` | 2 | 5 | 86 | 99 | 143 | 202 | 366 |
| `silver_push_events` | 3015 | 5639 | 7710 | 13988 | 30680 | 50062 | 60643 |
| `silver_repo_naming` | 3143 | 5856 | 8177 | 14852 | 32651 | 52899 | 64174 |
| `silver_star_events` | 1 | 2 | 10 | 15 | 18 | 32 | 47 |

Two numbers in that table are worth reading as evidence rather than filler.
`silver_events_deduped` reaches **64,313** — exactly the source's distinct-`id` count — so the
loader's deliberate 1,270-row at-least-once redelivery folds once, live on BigQuery, across
twenty-nine window boundaries. And `gold_events_enriched` matches it row for row and value for
value, which is the `LEFT JOIN`-against-a-`unique_key`-dimension shape agreeing across engines.

Machine-readable: `13-parity.json` (natural pair), `13-parity-attribution.json` (the
attribution run), `13-parity.md` (the two tables above, generated).

## 6 — The one divergence, and its attribution (D2, D4)

**`bronze_events`, at every intermediate checkpoint, always with `duck_only = 0`.**

*Root cause.* `bronze.events` is a whole-source passthrough — `materialization: table`, no
incremental strategy — so each window rebuilds it from whatever the source holds *at that
moment*. The two legs' sources do not arrive the same way: BigQuery's
`smelt_dogfood.github_events` was fully populated by phase 17 before window 1, while the
DuckDB leg's `load_day.sh` appends day D at window D. Identical behaviour over different
input. The gap closes monotonically — 62,382 → 59,605 → 57,217 → 50,406 → 32,251 → 11,536 →
**0** — and every DuckDB row is present on the BigQuery side at every checkpoint, never the
reverse.

*Attribution, measured rather than argued (D2's prescribed procedure).* The DuckDB leg was
replayed with the source staged up front — `run_incremental.py --preload-source`, which runs
`load_day.sh` for all thirty days before window 1 and then lets the loader's own per-day
idempotence turn the declared external step into a no-op inside each run. `load_day.sh` itself
is **unchanged**. Against that leg the sweep is clean:

```
w01 relations=14 divergent= []      w10 relations=14 divergent= []
w02 relations=14 divergent= []      w20 relations=14 divergent= []
w03 relations=14 divergent= []      w30 relations=14 divergent= []
w05 relations=14 divergent= []
```

**The divergence does not survive the attribution.** It is a statement about arrival order,
not about DuckDB or BigQuery. Nothing here is handed to `20260906-bigquery-correctness`.

*Registered, with a checkable bound.* `TARGET_DIVERGENCE_REGISTRY` holds exactly one entry,
for `bronze_events`, under a new `DivergenceBound::ArrivalLag { event_time_column,
behind_side }`. The bound is deliberately **not** "rows may differ": it requires (1) the
lagging leg holds nothing the leading leg lacks, and (2) every row only the leading leg holds
falls on or after the lagging leg's own maximum event-time day — so the difference is exactly
the tail that has not arrived, and a row lost from *inside* the lagging leg's loaded range
still fails the sweep. Clause (2) is what stops the entry from licensing a real row-loss bug,
and it is proved in both directions offline by
`an_arrival_lag_bound_rejects_a_lost_row_inside_the_loaded_range`.

## 7 — The structural exclusion (not a divergence)

`silver.actor_sessions` and `marts.daily_active_contributors` were excluded from **both** legs,
so the relation sets are equal by construction. Their absence from BigQuery is a compile-time
refusal, reproduced this session:

```
Error: UnsupportedOnBackend: this model uses 2 constructs the BigQuery backend cannot express:
  `LAG` — this dialect's RANGE window frames take a numeric offset over a numeric ORDER BY and
    have no INTERVAL form; state the frame numerically (e.g. ORDER BY UNIX_MICROS(ts) RANGE
    BETWEEN 172800000000 PRECEDING AND CURRENT ROW for two days), or use a ROWS frame
  `MAX` — [same]
```

`marts.daily_active_contributors` is its only downstream consumer, and that is checked from
the project rather than asserted from memory
(`excluded_models_are_exactly_the_compile_refused_pair` walks `models/` for
`smelt.silver.actor_sessions` references). A future silent widening of the exclusion list
fails that test rather than shrinking criterion 6's comparison quietly.

## 8 — Cost (stop gate: US$1)

Read from BigQuery job history (`statistics.query.totalBytesBilled`), not from dry runs.
2,160 jobs between 2026-09-11T21:50Z and the end of the phase, all `DONE`:

```
{"jobs": 2160, "totalBytesBilled": 58460209152, "GB": 58.4602, "usd_at_5_per_TB": 0.2923}
```

**US$0.29**, well inside the US$1 stop-and-report threshold and the dataset's US$25/month cap.
That covers everything: the 30 windows of pipeline execution, the 98 relation exports across
seven snapshots, the 18 drops (0 bytes each), the basis checks and the export-rate
measurement. At ~27 MB billed per job the great majority is BigQuery's 10 MB per-table
minimum-billing floor applied many times over, not data volume — the inputs are 65,583 rows.

## 9 — Gates

```
cargo test -p smelt-cli --test github_activity_dual_target
  -> 16 passed; 0 failed
cargo test -p smelt-cli --test github_activity_loader
  -> 11 passed; 0 failed
SMELT_BQ_DOGFOOD_LIVE=1 cargo test -p smelt-cli --features bigquery \
  --test github_activity_dual_target duckdb_and_bigquery_agree
  -> 1 passed  (the live sweep, with the registered bound holding against the real snapshots)
```

The offline sixteen include the six the plan asked for plus the four that keep the bound
vocabulary live:

- `relation_set_mismatch_fails` — coverage totality names the relation and the side.
- `an_unregistered_target_divergence_fails` — a real value difference fails with both counts.
- `the_sweep_fails_closed_on_an_empty_registry` — **rewritten** for the registry now being
  non-empty: `check_targets_agree_against` takes the registry as a parameter, so the control
  drives the real path over an *empty* registry regardless of what
  `TARGET_DIVERGENCE_REGISTRY` holds today. Adding or removing an entry can no longer retire
  it, which the previous `assert!(registry.is_empty())` form would have.
- `registry_entries_are_all_live` — **two-sided**, against the committed report: an entry
  naming a relation the report shows agreeing must be deleted; a relation the report shows
  diverging with no entry must be registered.
- `excluded_models_are_exactly_the_compile_refused_pair` — the exclusion set, checked against
  the project's own reference graph.
- `the_parity_report_covers_every_model_on_both_targets` — 14 relations at *every* checkpoint
  (a report that covered twelve at one and fourteen at another would make the ratchet vacuous
  for the two it dropped), no blank cell, final window present, neither excluded model listed.
- `the_two_targets_agree_at_the_final_window` — criterion 6's core claim, gated.
- `controlling_arrival_order_removes_every_divergence` — the attribution claim, gated against
  `13-parity-attribution.json` rather than left in this document's prose.
- `an_arrival_lag_bound_rejects_a_lost_row_inside_the_loaded_range`,
  `a_monotone_bound_holds_and_rejects_the_leading_side`, `a_bound_never_licenses_a_missing_row`
  — the bound vocabulary, both directions.
- `landing_a_bigquery_snapshot_casts_to_the_duckdb_legs_own_types`,
  `a_perturbed_landed_cell_is_still_reported`, `a_value_that_will_not_cast_is_a_loud_failure`,
  `bookkeeping_relations_are_excluded_on_both_sides` — the landing seam.

`bash .claude/scripts/verify-phase.sh` output is quoted in the commit for this phase.

## Findings

1. **The two targets agree, and the one difference is not about the engines.** Fourteen of
   fourteen relations byte-equal at the final window; thirteen of fourteen at *every*
   checkpoint; the fourteenth explained and removed by controlling arrival order. Nothing to
   escalate to `20260906-bigquery-correctness` from this phase.
2. **`bronze.events` is the pipeline's only arrival-order-sensitive model**, because it is the
   only one that is a whole-source rebuild rather than window-addressed or keyed. That is a
   fact about the example, not a defect: every window-addressed and keyed model is
   window-limited on both targets, which is precisely what the intermediate checkpoints prove.
3. **A concurrent `cargo` invocation can silently de-feature a long live run.** `cargo test -p
   smelt-cli` (no `--features bigquery`) overwrote `target/debug/smelt` mid-leg and the next
   window refused at backend construction. Anything driving a live warehouse for an hour
   should run against a pinned binary copy, not `target/debug/`. `scripts/bq-dogfood-parity.sh`
   now honours `SMELT_BIN` and `PARITY_RESUME_FROM` for exactly this.
4. **An impersonated token outlives less than the leg does.** The thirty-window BigQuery leg
   takes ~61 minutes against a one-hour token, so the script re-mints before every window and
   every snapshot via `PARITY_TOKEN_CMD` rather than relying on one token exported up front.

## What phase 14 can assume

- **Criterion 6 is met.** `13-parity.json` is the measured evidence, `13-parity-attribution.json`
  the arrival-order control, and both are gated per-PR by
  `cargo test -p smelt-cli --test github_activity_dual_target`.
- `smelt_dogfood` holds the **14 model tables rebuilt over the full thirty-day population**,
  row-for-row equal to the DuckDB leg's, plus the two untouched source tables (65,583 rows
  each), the two empty `__tombstones` tables, `_smelt_ledger` (270 rows) and
  `_smelt_observed_delta` (29 rows). Engine-resident bookkeeping is being written on BigQuery
  now — that is new since phase 12, and those four tables are **excluded** from the parity
  comparison by prefix/suffix, for the reason the DuckDB oracle already excludes them.
- The BigQuery interval frontier is `2026-08-05 → 2026-09-04` for every interval-addressed
  model, one interval, `cell_frontiers: {}`.
- **The schedule is the fixture's thirty windows**, and the two excluded models are
  `silver.actor_sessions` and `marts.daily_active_contributors` — a compile-time
  `UnsupportedOnBackend` refusal on the INTERVAL `RANGE` frame, not a value divergence. Phase
  14's oracle leg must exclude the same pair or it will be comparing different model sets.
- **Arrival order is a controlled variable now, with a mechanism**:
  `run_incremental.py --preload-source` stages the whole source before window 1 and is the
  arrival order BigQuery's warehouse-resident source has. Phase 14's full-refresh oracle over
  BigQuery should use it if it needs the two legs' *intermediate* states to match, and may
  ignore it if it only compares each target against its own oracle.
- **`probes: { cadence: off }` is gone and stayed gone** — one committed configuration served
  both targets for all thirty windows, with probe dispatch at its default cadence throughout.
- The snapshots themselves live under `target/phase13/` (gitignored): seven DuckDB database
  files per leg and 98 NDJSON exports, ~250 MB. Regenerate with
  `bash scripts/bq-dogfood-parity.sh {clear,duck,duck-preloaded,bq,manifest,report}`.
- Hard deadline **2026-09-19** still stands for phase 14: the source tables' oldest partition
  (2026-08-05) expires then, and a comparison after it is invalid rather than merely late.

## Not done / blocked

- Nothing was blocked.
- **Thirty full checkpoints were not run**; seven were, and the reduction is measured and
  declared in §3 rather than assumed. A thirty-checkpoint sweep is feasible — it would have
  added ~65 minutes of export — and needs only `PARITY_CHECKPOINTS=1,2,3,…,30`.
- No fix was made to anything (this phase's plan §"What this phase does not do"), and nothing
  needed one.
- Nothing outside `smelt_dogfood` was written; `target: dev` was never unpinned.
