# Phase 12 summary — a full refresh and three consecutive incremental windows on BigQuery

**Ran against:** project `smelt-bq-test-20260816`, dataset `smelt_dogfood`, via ADC
impersonation of `smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`. Nothing
outside `smelt_dogfood` was touched; `github_events` and `github_events_arrival` (the loaded
history) were read only — never dropped, truncated or reloaded, and no statement in this
phase touched `githubarchive`.

**Headline: the T5 block is genuinely lifted, and the pipeline now runs incrementally on
BigQuery.** A clean full refresh plus **three consecutive incremental windows** completed,
each with a run report, each exit 0. `silver.events_deduped` — the model phase 11 died on —
succeeded on every one of the four runs. But criterion 5 is met **with two caveats that are
themselves the phase's main findings**: six of sixteen models are excluded from the runs
because they emit SQL BigQuery cannot parse, and probe dispatch had to be turned off for any
run to complete at all. Both are characterised below, neither is fixed here.

## What ran

| step | invocation (all with `--target bigquery`, six models excluded — see finding 2/3) | result |
|---|---|---|
| FR | `--full-refresh --start 2026-08-04 --end 2026-08-05` | `smelt: built 10 model(s) in 14.56s`, exit 0 |
| W1 | `--start 2026-08-05 --end 2026-08-06` | `smelt: built 10 model(s) in 13.94s`, exit 0 |
| W2 | `--start 2026-08-06 --end 2026-08-07` | `smelt: built 10 model(s) in 14.14s`, exit 0 |
| W3 | `--start 2026-08-07 --end 2026-08-08` | `smelt: built 10 model(s) in 13.18s`, exit 0 |

The windows were derived from the table, not assumed. Read back this session by
`GROUP BY` over each partition column:

```
events           2026-08-04  75     arrival_ingested 2026-08-05 3276
events           2026-08-05  3264   arrival_ingested 2026-08-06 2777
events           2026-08-06  2714
```

Three event-time days exist and no more (phase 10 loaded 08-05 and 08-06; 08-04 is day-05's
2% redelivery reach-back), so the schedule is one full-refresh day plus three consecutive
incremental days. **W3 lands no new source rows on purpose**: an empty window is exactly the
shape that makes a keyed merge suppress a no-op write, which is the path the T5 downgrade
now sits on. A fourth window would also be empty and would add nothing.

### The models that ran

Ten, in every run: `bronze.events`, `silver.events_deduped`, `silver.repo_naming`,
`silver.actor_naming`, `silver.push_events`, `silver.pr_events`, `silver.issue_events`,
`silver.star_events`, `gold.repo_activity_daily`, `marts.naming_history`.

Six excluded: `gold.repo_dim` and `silver.actor_sessions` (they fail on emitted SQL —
findings 2 and 3), plus `gold.events_enriched`, `marts.daily_active_contributors`,
`marts.repo_leaderboard`, `marts.star_growth`, which are downstream of them. The exclusion
set is not arbitrary: `smelt run -e` refuses a torn working set loudly, which is how the
last two were identified —

```
Error: Inconsistent working set after --exclude: the following retained models have missing
upstream dependencies:
  model 'marts.star_growth' requires upstream 'gold.events_enriched' which was excluded
  model 'marts.repo_leaderboard' requires upstream 'gold.repo_dim' which was excluded
```

## The numbers, read back from BigQuery after each run

`SELECT table_id, row_count FROM smelt_dogfood.__TABLES__` after each step:

| table | after FR | after W1 | after W2 | after W3 |
|---|---|---|---|---|
| `bronze_events` | 6,053 | 6,053 | 6,053 | 6,053 |
| `silver_events_deduped` | 75 | 3,276 | 5,990 | 5,990 |
| `silver_push_events` | 71 | 3,086 | 5,710 | 5,710 |
| `gold_repo_activity_daily` | 37 | 436 | 799 | 799 |
| `silver_repo_naming` / `silver_actor_naming` | 6,053 | 6,053 | 6,053 | 6,053 |
| `silver_pr_events` / `issue_events` / `star_events` | 0 / 1 / 0 | 2 / 2 / 1 | 5 / 2 / 2 | 5 / 2 / 2 |
| `marts_naming_history` | 2 | 2 | 2 | 2 |

Two of these are worth stating as *evidence rather than colour*:

- **The dedup is exactly right after W2.** `silver_events_deduped` reaches **5,990**, which
  is precisely the distinct-`id` count of the whole source (6,053 rows, 5,990 distinct ids —
  phase 10 measured the same 63 duplicates). The at-least-once redelivery the loader injects
  on purpose is folded once, live on BigQuery, across window boundaries. The intermediate
  W1 figure closes the same way: 3,276 = 3,201 real day-05 rows + 75 redelivered day-04 rows.
- **W3, the empty window, changed nothing.** Every row count is byte-identical to W2's. A
  no-op window is a no-op write, which is what the downgraded plan promises.

## Run reports — captured, and only because the project now opts into state

Phase 11 recorded that no run report existed for this project: with no `state:` key it runs
at the default `state.mode: stateless`, which writes nothing (`docs/specs/run_state.md`
§"Stateless writes nothing"). This phase adds `state: { mode: intervals }` to
`examples/github_activity/smelt.yml` — the posture is the deliverable here, not an
incidental — and the reports appear. W2's, verbatim and complete:

```json
{
  "run_id": "20260911-034108-f3d1e2",
  "started_at": "2026-09-11T03:41:08.735413341Z",
  "completed_at": "2026-09-11T03:41:22.869848375Z",
  "duration_ms": 14134,
  "outcome_counts": { "success": 10, "failed": 0, "skipped": 0 },
  "failures": [],
  "external_steps": {
    "sources.raw.github_loader": {
      "command": ["bash", "load_day.sh", "--date", "2026-08-06"],
      "produces": ["smelt.sources.raw.github_events", "smelt.sources.raw.github_events_arrival"],
      "duration_ms": 16, "outcome": "success"
    }
  }
}
```

The other three are the same shape: `20260911-033954-5152d4` (FR, 14,555 ms),
`20260911-034038-a8b6fd` (W1, 13,937 ms), `20260911-034138-938fe9` (W3, 13,174 ms), all
`{"success": 10, "failed": 0, "skipped": 0}`. They live under
`examples/github_activity/.smelt/targets/bigquery/reports/`, which is gitignored — hence
quoting them here.

Note the `external_steps` entry: the loader step reports `success` on the BigQuery target
while actually doing nothing for it (`load_day.sh: day 2026-08-06 already loaded, skipping`,
against the *local DuckDB file*). That is phase 11's finding 2 (`external_step:` has no
target-awareness) now visible in the run report as a success that is not one.

## Frontier and engine-resident state, inspected between runs

**The `.smelt/` frontier advanced monotonically, one day per window** — read from
`intervals.json` after each run, not inferred:

| after | `gold.repo_activity_daily` covered interval |
|---|---|
| FR | `2026-08-04 → 2026-08-05` |
| W1 | `2026-08-04 → 2026-08-06` |
| W2 | `2026-08-04 → 2026-08-07` |
| W3 | `2026-08-04 → 2026-08-08` |

The four typed fan-out models carry the identical coverage, each with its own `model_hash`.
Consecutive windows merge into one interval rather than accumulating fragments, which is
the interval ledger behaving as specced. `cell_frontiers` is `{}` throughout (no
`contract.deferral` cell in this project). `landed_deltas.json` is `{}` after every run —
worth knowing for the forward-propagation work, since `--since-upstream` reads exactly that.

The interval ledger holds **only** the five window-addressed models. The succession models
(`repo_naming`, `actor_naming`), the keyed dedup and `marts.naming_history` have no entry:
their strategies (`full_refresh`, `cumulative_aggregate`) are not interval-addressed. So on
BigQuery the only frontier an operator can inspect for a keyed model is *nothing at all* —
which follows from the next paragraph rather than being a bug.

**The engine-resident state is the absence the degradation contract promises.** After four
runs, `smelt_dogfood` holds exactly the two source tables plus the ten model tables — no
merge ledger, no reconciliation ledger, no `*__tombstones` sibling, no fingerprint sidecar,
no observed-delta table. That matches `docs/specs/state.md` §"Which dialects realise which
structure" (BigQuery: "not yet" on all five rows) and is what `20260906-bigquery-correctness`
phase 11 made honest. The consequence is visible in the manifest's own `strategy` column:
`silver.repo_naming`/`actor_naming` run `full_refresh` rather than `succession_patch`,
`silver.events_deduped` runs `cumulative_aggregate`, and the four static
`MaintenanceStateDowngraded` diagnostics
(`crates/smelt-cli/tests/example_diagnostics/smoke_and_migration.rs:54-73`) say why.

## Findings

### 1. (Primary) The append-only posture probe cannot be planned by BigQuery at all — two defects compounding

**Provoking model/statement:** `silver.repo_naming`, the `SourceMutationProfileViolated`
probe over `raw.github_events`. Verbatim:

```
smelt: run failed at model 'silver.repo_naming': Execution failed for 'silver.repo_naming':
Failed to execute SourceMutationProfileViolated probe for model 'silver.repo_naming':
  SQL: WITH __append_only_violations AS (SELECT CAST(__current.partition_value AS STRING) …
  Error: Execution failed for 'bigquery sql': BadRequest: 400 …: Resources exceeded during
  query execution: Not enough resources for query planning - too many subqueries or query is
  too complex.
```

The emitted statement is **692,597 characters**. Two independent defects produce it:

- **(a) The posture baseline ignores the source's declared granularity.**
  `emit_append_only_baseline_snapshot`
  (`crates/smelt-logical/src/maintenance/emit/probes.rs:701-733`, the `GROUP BY
  {partition_column}` at line 731) groups by the **raw** partition column.
  `raw.github_events` declares `partition_column: created_at` with `granularity: day` over a
  TIMESTAMP, so the recorded baseline holds one "partition" per distinct **second**. Measured
  from the state file smelt wrote:
  `{'raw.github_events': 5797, 'raw.github_events_arrival': 2}` — 5,797 recorded partitions
  for a three-day source. The arrival twin gets 2 only because `ingested_date` is already a
  DATE; a DATE partition column hides the bug, a TIMESTAMP one exposes it. This is wrong on
  every backend — the "closed partition" reasoning behind the append-only late-arrival
  classification is being done per-second instead of per-day — DuckDB just never complains.
- **(b) BigQuery's inline row set is one subquery per row.**
  `build_row_set_table`/`row_set_body` (`crates/smelt-core/src/sql/row_set.rs:55-71`) emit
  `VALUES …` for DuckDB/Spark and a chained `SELECT … UNION ALL SELECT …` for BigQuery. That
  is correct SQL and it does not scale: 5,797 branches exceed the GoogleSQL planner's
  complexity limit. GoogleSQL's own scalable form is
  `UNNEST(ARRAY<STRUCT<…>>[…])`, one operand regardless of row count.

**No state hygiene avoids it.** Deleting the recorded baseline between runs was tried and
failed identically: one model *establishes* the baseline and the next *verifies* against it
**inside the same run** (`crates/smelt-runtime/src/source_probes.rs:110-170`). So with probe
dispatch on, this pipeline cannot complete a single run against BigQuery — the failure is not
a second-run phenomenon.

**Owner:** `20260906-bigquery-correctness`. (a) is a plan-layer fix and benefits every
backend; (b) is the BigQuery row-set spelling. Either alone would have avoided this stop.

### 2. `FILTER (WHERE …)` is emitted verbatim on BigQuery, which has no such clause

**Provoking model/statement:** `gold.repo_dim`, `MAX(repo_name) FILTER (WHERE is_current)
AS current_repo_name`.

```
smelt: run failed at model 'gold.repo_dim': Execution failed for 'bigquery sql':
BadRequest: 400 Syntax error: Expected ")" but got "(" at [38:27]; reason: invalidQuery
```

`[38:27]` is the `(` after `FILTER` in the compiled statement (confirmed with `-v`). GoogleSQL
has no aggregate `FILTER` clause; the portable lowering is `MAX(CASE WHEN is_current THEN
repo_name END)` or `MAX(IF(is_current, repo_name, NULL))`. smelt does hold a `FILTER`-aware
refusal — but only for *template*-spelled built-ins
(`crates/smelt-dialect/src/emission_check.rs:53-131`) and for the restructure path
(`crates/smelt-dialect/src/restructure.rs:307-321`). A plain aggregate carrying `FILTER`
has neither a lowering nor a capability flag: `BackendCapabilities`
(`crates/smelt-dialect/src/dialect.rs:60-184`) has 23 `supports_*` flags and none of them is
about `FILTER`. So this reaches the warehouse and fails there rather than being refused at
compile time by `dialect_seam`.

**Owner:** `20260906-bigquery-correctness`. Either a lowering (preferred — the `CASE` form is
exactly equivalent) or a capability flag plus an `UnsupportedOnBackend` refusal.

### 3. An `INTERVAL` window frame is emitted verbatim on BigQuery, which allows only numeric RANGE offsets

**Provoking model/statement:** `silver.actor_sessions`,
`LAG(created_at) OVER (PARTITION BY actor_id ORDER BY created_at RANGE BETWEEN INTERVAL
'2 days' PRECEDING AND CURRENT ROW)` — the model's declared `max_lookback`.

```
smelt: run failed at model 'silver.actor_sessions': Execution failed for 'bigquery sql':
BadRequest: 400 Syntax error: Unexpected keyword PRECEDING at [38:49]; reason: invalidQuery
```

GoogleSQL's `RANGE` frames take a numeric offset over a numeric `ORDER BY`; there is no
`INTERVAL` form. The printer has frame *analysis* (`crates/smelt-dialect/src/position.rs:
172-204` classifies running vs whole-partition frames) but no per-dialect frame
*capability*, so the frame is printed as written.

**A third defect hides in the same statement**, which would surface the moment the frame is
fixed: the compiled SQL contains `LAG(CAST(NULL AS VARCHAR))`. `VARCHAR` is not a GoogleSQL
type name (`STRING` is). A dialect-blind type spelling on the null-placeholder path — same
class as the `key_expr_for_columns` hardcoded `CAST(... AS VARCHAR)` that
`20260906-bigquery-correctness` phase 2 already fixed elsewhere, so this is a *missed site*
of a known bug rather than a new class.

**Owner:** `20260906-bigquery-correctness`.

### 4. The precision downgrade is invisible at run time — nothing says the observed delta was skipped

The T5 path now degrades instead of refusing, which is the whole reason this phase could run.
But the degradation is **silent**: the four runs' console output is two lines each, no warning
is emitted, and the run report has no field for it. The four static
`MaintenanceStateDowngraded` diagnostics cover *technique* downgrades (the "losing a
technique" half of `docs/specs/state.md` §"The degradation contract"); the "losing precision"
half — the observed output delta not being recorded — is surfaced nowhere an operator would
see. `smelt explain` still takes no `--target` (phase 11 finding 3), so there is not even an
offline way to ask what a BigQuery run would downgrade.

The spec says the degradation is "recorded, explain-visible". For this class, on this
backend, it is neither. **Owner:** `20260906-bigquery-correctness` (it owns the reconciliation
of the two layers), possibly with a `docs/specs/state.md` clarification about what "recorded"
means for a precision loss.

### 5. No single committed configuration serves both targets

The live BigQuery leg needs `probes: { cadence: off }` (finding 1). With that set, the DuckDB
gate `github_activity_replay::recurrence_bound_violation_fails_the_run` — the negative control
that a duplicate pair violating the declared zero-width `key_recurrence` **fails the run** —
goes green when it should fail:

```
thread 'recurrence_bound_violation_fails_the_run' panicked at
crates/smelt-cli/tests/github_activity_replay.rs:44:5:
expected smelt run [2026-08-06 .. 2026-08-07) to fail, but it succeeded
```

Red-green confirmed both ways: that test fails with `cadence: off` and passes with it removed,
everything else unchanged. So the project can have a runnable BigQuery leg **or** a real
recurrence-bound negative control, not both. The committed `smelt.yml` therefore keeps probes
on and carries the `probes:` block **commented out** with the reason; phase 12's live windows
ran with it uncommented, and that two-line diff is the exact reproduction recipe. Filed rather
than papered over, per this outcome's "characterise, do not fix".

This is also, independently, a fail-loud observation: `cadence: off` silently converts a
*negative control* into a passing test. The manifest does record each probe as
`"outcome": "skipped"`, which is honest — but nothing at the test layer notices that a run it
expected to fail only succeeded because the check was not dispatched.

### 6. An empty incremental window costs as much as a full one

W3 landed no new rows and changed no table, and billed **230,686,720 bytes — the same as W2**,
which landed 2,714 events. Every model still re-scanned its inputs. On the 10 MB-per-table
minimum-billing floor this is noise, but on a real-sized table it is not: an idle daily run of
a downgraded plan costs a working day's scan. Recorded as an operational fact for
`20260906-bigquery-unattended`, which will schedule exactly such runs.

## Cost

Read from BigQuery job metadata (`bigquery/v2/.../jobs`, `statistics.query.totalBytesBilled`),
not from dry runs. 258 jobs between 03:29:31Z and 03:42:04Z on 2026-09-11, bucketed by each
run's own wall clock:

| run | jobs | `totalBytesBilled` | cost @ $5/TB |
|---|---|---|---|
| FR1 (first attempt, probe failure) | 7 | 10,485,760 | $0.00005 |
| FR2 (baseline pruned, same failure) | 18 | 41,943,040 | $0.00021 |
| FR3 (probes off; the two syntax failures) | 43 | 115,343,360 | $0.00058 |
| FR (clean, 10 models) | 32 | 115,343,360 | $0.00058 |
| W1 | 38 | 209,715,200 | $0.00105 |
| W2 | 38 | 230,686,720 | $0.00115 |
| W3 (empty window) | 37 | 230,686,720 | $0.00115 |
| **whole session, including inspection queries and drops** | **258** | **1,111,490,560 (1.111 GB)** | **≈ $0.0056** |

Every job hit the 10 MB per-table minimum-billing floor — the inputs are ~6,053 rows — so
these are floor charges, not data volume. Nothing in this phase read `githubarchive`; the
loader was not run. Well under a cent, and far under both the AUD 25/month budget and the
~US$1 stop-and-report threshold.

Two incidental job-history observations: 30 `DROP_MATERIALIZED_VIEW` jobs and 26 jobs with
`errorResult.reason: "invalid"` appear across the session — smelt's create-or-replace path
probes for a materialized view that does not exist and absorbs the error. Zero-billed and
harmless, but it means "errored jobs" in this project's history is not a useful health signal.

## What was changed in the repo

Only one file under `examples/`, plus this phase's documents. **Nothing under `crates/`** —
no production code, no test, no baseline, no ratchet.

- `examples/github_activity/smelt.yml`: adds `state: { mode: intervals }` (with a comment
  explaining that run reports and the interval frontier are the live evidence, and that the
  reconciliation ledger is *not* what this key controls), and a commented-out `probes:
  { cadence: off }` block carrying finding 1 and finding 5 inline.

## Gates

With the committed configuration (intervals on, `probes:` commented out):

- `cargo test -p smelt-cli --test github_activity_replay` — **21 passed**, 0 failed.
- `cargo test -p smelt-cli --test github_activity_oracle` — **18 passed**, 1 ignored.
- `cargo test -p smelt-cli --test example_diagnostics` — **128 passed**, 1 ignored.
- `cargo test -p smelt-cli --test github_activity_loader` — **11 passed**.
- `cargo test -p smelt-lsp --test example_workspaces github_activity` — **1 passed**.
- `cargo fmt --all` — clean (no Rust file was touched; run to prove it rather than assumed).
- The full workspace suite was **not** run, deliberately: no production code changed. Stated
  rather than skipped silently.

## What was NOT done

- No fix under `crates/`. Findings 1-4 are all live, all characterised, all handed to
  `20260906-bigquery-correctness`.
- Six of sixteen models never executed on BigQuery this phase (findings 2 and 3), so
  **phase 13's dual-target parity can compare ten models, not sixteen** — `gold.repo_dim`,
  `gold.events_enriched`, `silver.actor_sessions`, `marts.repo_leaderboard`,
  `marts.star_growth` and `marts.daily_active_contributors` have no BigQuery side to compare
  until those two emission defects are fixed.
- No full-refresh oracle comparison (phase 14's job); the BigQuery side is left populated
  and at a known window frontier (`2026-08-04 → 2026-08-08`) for phases 13 and 14 to read.
- The four derived tables phase 11 left behind were dropped, as authorised, to clear the
  documented `SourceRetentionExceeded` refusal on a repeat `--full-refresh`; the two loaded
  source tables were never touched and both still hold 6,053 rows.

## What phase 13 can assume

- `smelt_dogfood` holds ten model tables built by a full refresh plus three incremental
  windows, at frontier `2026-08-04 → 2026-08-08`, plus the two untouched source tables.
- The run reports, run manifests, interval ledger and per-model deployed schemas exist under
  `examples/github_activity/.smelt/targets/bigquery/` (gitignored — regenerate by re-running,
  or read the figures quoted above).
- Reaching the warehouse needs: `cargo build -p smelt-cli --features bigquery`,
  `source scripts/bq-dogfood-env.sh`, and
  `export SMELT_BQ_ACCESS_TOKEN=$(gcloud auth application-default print-access-token)` — the
  last one still undocumented anywhere but phase 11's summary and this one. Always pass
  `--target bigquery`; never unpin `target: dev`.
- A BigQuery run of this project **will not complete** unless `probes: { cadence: off }` is
  uncommented in `examples/github_activity/smelt.yml` (finding 1). Re-comment it before
  running the DuckDB gates (finding 5).
