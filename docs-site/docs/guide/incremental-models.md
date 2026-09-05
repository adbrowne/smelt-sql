# Incremental Models

Incremental materialization lets you process only new or changed data instead of rebuilding an entire table from scratch. For large datasets, this can reduce run times from hours to seconds.

## What a model emits

Between one run and the next, something changed upstream — new rows arrived, an existing row was
corrected. That change is a **delta**, and every relation in your pipeline — a source or a
model — has a **delta signature**: the shape of change it emits when it changes.

Not every delta is equally hard to absorb. smelt grades the shape of a change on a three-point
scale, from easiest to hardest:

`append-only within a window` ⊑ `keyed upsert` ⊑ `general`

- **append-only within a window** — only new rows, landing inside a bounded, recent time window.
  An event feed is the canonical case.
- **keyed upsert** — rows are added or replaced, addressed by a key. The output of a keyed
  aggregation is the canonical case.
- **general** — anything: inserts, updates, deletes, anywhere. The least that can be assumed,
  and the most expensive to absorb.

A signature's **addressing** says how the changed rows are located: by time **window**, by
**key set**, or only as **whole-table**. Run `smelt explain <model>` to see the derived
signature for one of the timeseries example project's models:

```
$ smelt explain user_daily_spend
model user_daily_spend  (emits: keyed upsert over [user_id, spend_date], key-addressed, slice-bounded by spend_date under key temporal locality (settle bound: After { margin: Seconds(0) }); grain: partition)
```

That headline is the model's own delta signature — a keyed upsert over `[user_id, spend_date]`,
narrowed to the `spend_date` window a given run touches. **A model's signature is derived from
its SQL and its inputs' signatures — you never declare it.** A keyed `GROUP BY` over an
append-only feed emits keyed upserts; a row-multiplying join degrades what flows through it
toward `general`. `smelt explain` names the construct responsible whenever a model degrades:
run it against `daily_events_enriched` in the same project and the headline reads
`emits: general (degraded by: join proven OneToMany (row-multiplying) degrades output-delta
shape to General), not delta-addressable` — the plain inner join is exactly the shape that
can't be proven row-preserving (more on this in
[Enrichment joins and dimension updates](#enrichment-joins-and-dimension-updates) below).

What you actually declare are two shape-defining facts about the *output* — a clock, an
identity, or both — covered next in [Configuration](#configuration). From there: the
[composed shape](#the-composed-shape-key-time) when you declare both facts together,
[contract relaxations](#contract-relaxations) if a column doesn't need strict equivalence,
[Rebuilding](#rebuilding) for backfills and definition changes, and
[Schema Evolution](schema-evolution.md) for adding or removing columns.

## Configuration

`refresh: incremental` is admitted by the **shape-defining facts** you declare, not by a separate mode selector: a `timeseries:` block (the **clock**) and/or a top-level `unique_key:` (the **identity**). Declaring the clock alone gives you the partition-addressed shape this guide covers — one row per `(partition_column, …)`; declaring the identity instead gives you the key-grain shape (see [Materializations](materializations.md#refresh-axis) and the [key-grain patterns reference](../reference/cumulative-aggregate.md)). Declaring **both** together is a shape of its own — a key-addressed table that is also time-partitioned — covered under [The composed shape](#the-composed-shape-key-time) below. `refresh: incremental` with neither fact declared is a hard error naming what's missing.

- **`refresh: incremental`** opts the model into the derived maintenance plan. It implies a stored `table` — you do not also declare `materialization: table`.
- **`timeseries:`** declares the clock — which column is the event time, which column partitions the output, and at what granularity. See the [timeseries reference](../reference/timeseries.md) for the full key table.
- **`grain: partition`** is the friendly name for this shape — an *optional* check-only assertion, not what admits the model. Write it if you want it in frontmatter; it errors if it disagrees with what the declared facts derive.
- **`safety_overrides:`** (optional, top level) overrides safety checks for patterns that may behave differently on partial data — see [Safety analysis](#safety-analysis). Admitted only on a partition-shaped output (a declared clock, no declared identity).

`timeseries:` is required for the shape this guide covers (a model with no declared identity). Declaring `refresh: incremental` without any shape-defining fact is a validation error (`GrainRequiredForIncremental`).

### Frontmatter example

```sql
---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: transaction_timestamp
  partition_column: revenue_date
  granularity: day
---

SELECT
    CAST(transaction_timestamp AS DATE) as revenue_date,
    user_id,
    COUNT(*) as transaction_count,
    SUM(amount) as total_revenue
FROM smelt.sources.raw.transactions
WHERE transaction_timestamp IS NOT NULL
GROUP BY 1, 2
```

`refresh: incremental` + `grain: partition` opts the model into incremental execution; `timeseries:` declares the time dimension.

### smelt.yml example

```yaml
models:
  daily_revenue:
    materialization: table
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: transaction_timestamp
      partition_column: revenue_date
      granularity: day
```

### Partition-grain frontmatter keys

| Key | Required | Description |
|---|---|---|
| `safety_overrides` | No | Override safety checks for patterns that may behave differently on partial data. See [Safety analysis](#safety-analysis). |
| `columns.<c>.contract` | No | Set to `plausible` to exempt output column `<c>` from the determinism requirement (e.g. an `inserted_at = NOW()` audit stamp). See [Non-deterministic columns](#non-deterministic-columns). |

For the `timeseries:` fields (`event_time_column`, `partition_column`, `granularity`, `week_start`), see the [timeseries reference](../reference/timeseries.md).

### Granularity options

- **`hour`** -- One partition per hour. Use for high-frequency data.
- **`day`** -- One partition per calendar day. The most common choice.
- **`week`** -- One partition per week. Supports custom week start day:
  ```yaml
  granularity:
    week:
      week_start: monday
  ```
- **`month`** -- One partition per calendar month.
- **`quarter`** -- One partition per calendar quarter.
- **`year`** -- One partition per calendar year.

### Renaming `partition_column`

Once a partition-grain model's table has been deployed, changing `timeseries.partition_column`
is refused: `smelt run` (and `smelt build`) reports `MaintenancePartitionColumnChanged`, naming
the recorded and the current column. This refusal fires even for a rename that repoints the
address at a column your query already projects — a column-set diff alone can't see it, but the
partition address is still a world fact that changed. No run flag bypasses it (the analyzer gate
refuses on any Error-severity diagnostic unconditionally). To accept the rename, delete the
model's recorded snapshot at `.smelt/targets/<target>/schemas/<model>.json` and re-run — the
next run addresses the table under the new column and records a fresh snapshot.

## Running incremental models

### What a partition-shaped run does

For a partition-shaped model (a declared clock, no declared identity — the shape this guide
covers), the maintenance plan derives to the region-recompute technique on the model's own
`NewData` trigger: **DELETE**+**INSERT** on time partitions. For each run touching a partition:

1. **DELETE** rows in the output table where the partition column falls within the requested time range.
2. **Run the query** with a WHERE filter restricting source data to that time range.
3. **INSERT** the results into the output table.

This is idempotent -- running the same time range twice produces the same result, because the
DELETE step clears any previous output before inserting. It is the technique the derived plan
assigns to this trigger for this shape, not a strategy you choose: a keyed or composed model,
or a mutation triggered by an upstream dimension change, can derive a different technique for a
different cell of the same plan (see [Enrichment joins](#enrichment-joins-and-dimension-updates)
and [The composed shape](#the-composed-shape-key-time)). Run `smelt explain <model>` on your own
model to see which technique its own plan actually assigned.

### Explicit time range

Specify the start (inclusive) and end (exclusive) dates:

```bash
smelt run --start 2025-01-01 --end 2025-01-08
```

The longer form `--event-time-start` and `--event-time-end` is also supported and behaves identically.

!!! note
    Non-incremental models in the DAG are still executed normally. The time range only affects models with incremental configuration.

### Run window granularity vs partition granularity

The declared `granularity:` must be at least as coarse as the granularity actually produced by the model's `partition_column` expression. If `partition_column` is computed with `DATE_TRUNC('day', event_time)`, the model's real partition grid is daily, and `granularity: hour` is rejected even though the run window itself might otherwise look valid — an hourly run window doesn't correspond to a real partition boundary on a daily-partitioned model. The rejection names the minimum valid window:

```
run window granularity (hour) is finer than partition column 'event_date''s derived
granularity (day); the minimum run window for this model is one day
```

Declaring `granularity: day` (or coarser, e.g. `week`) on the same model is valid — run-window size and partition granularity are otherwise independent, as long as the declared granularity is not finer than what the SQL actually produces. When smelt can't determine the partition column's actual granularity from the SQL (an opaque function call it doesn't recognize), this extra check is skipped and only the ordinary run-window alignment check applies — smelt never guesses or silently widens the window to compensate.

### Auto mode

Let smelt determine what needs processing based on previously recorded intervals:

```bash
smelt run --auto
```

In auto mode, smelt reads the interval coverage from its state store and processes only the gaps -- time ranges that have not yet been successfully materialized.

### Per-partition execution

Force one query per granularity period instead of batching:

```bash
smelt run --start 2025-01-01 --end 2025-01-31 --per-partition
```

With daily granularity, this runs 30 separate DELETE+INSERT cycles, one for each day. Useful when you need strict per-day isolation or when queries are too large to process in bulk.

### Batch size override

Control how large each batch chunk is (in days):

```bash
smelt run --start 2025-01-01 --end 2025-04-01 --batch-size 7
```

This processes the 3-month range in weekly chunks of 7 days each.

## Complete example

This example walks through the `daily_revenue` model from the timeseries example project.

**The SQL model** (`models/daily_revenue.sql`):

```sql
SELECT
    CAST(transaction_timestamp AS DATE) as revenue_date,
    user_id,
    COUNT(*) as transaction_count,
    SUM(amount) as total_revenue,
    AVG(amount) as avg_transaction_amount,
    MIN(transaction_timestamp) as first_transaction,
    MAX(transaction_timestamp) as last_transaction
FROM smelt.sources.raw.transactions
WHERE transaction_timestamp IS NOT NULL
GROUP BY 1, 2
ORDER BY 1, 2
```

**The configuration** (in `smelt.yml`):

```yaml
models:
  daily_revenue:
    materialization: table
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: transaction_timestamp
      partition_column: revenue_date
      granularity: day
```

**Running it**:

```bash
# Process a single day
smelt run --select daily_revenue --start 2025-01-15 --end 2025-01-16

# Process a full month
smelt run --select daily_revenue --start 2025-01-01 --end 2025-02-01

# Catch up on any missing intervals
smelt run --select daily_revenue --auto

# Check what has been processed
smelt status daily_revenue
```

## Safety analysis

smelt statically analyzes your SQL for patterns that can produce incorrect results when run on partial (time-sliced) data. These patterns are blocked by default.

| Pattern | Why it is unsafe |
|---|---|
| Window functions with unbounded frames | A `ROW_NUMBER() OVER (ORDER BY ts)` computed on one day's data gives different results than the same function on the full dataset. |
| HAVING with aggregates | `HAVING COUNT(*) > 10` may filter out groups that would pass if all data were present — **unless** the `GROUP BY` already groups by `partition_column` (see below). |
| LIMIT | `LIMIT 100` on partial data returns different rows than on the full table. Always blocked — there is no group-alignment relaxation for it. |
| Non-deterministic functions | `RANDOM()`, `NOW()`, and similar functions produce different results on each run. |
| Subqueries | Subqueries may reference data outside the filtered time range. |
| DISTINCT | `SELECT DISTINCT` on partial data may miss duplicates that span partition boundaries — **unless** `partition_column` is projected in the same scope (see below). |

### Group-aligned HAVING and DISTINCT

`HAVING` and `DISTINCT` are admitted without a safety override when they are **group-aligned** to `partition_column`:

- A `HAVING` clause is admitted when its own `GROUP BY` is a superset of `partition_column` — every group is already scoped to one partition, so a partial run and a full refresh see the same group composition within that partition.
- A `SELECT DISTINCT` is admitted when `partition_column` is projected in the same scope — the dedup key is the whole row, so two rows can only collide when they share the same partition value.

```sql
-- Admitted without an override: GROUP BY includes partition_column (revenue_date).
SELECT
    transaction_timestamp::DATE as revenue_date,
    user_id,
    SUM(amount) as total_revenue
FROM raw.transactions
GROUP BY 1, 2
HAVING SUM(amount) > 100
```

This alignment check runs per scope: in a model whose SQL is a `UNION ALL` of several branches, each branch's own `HAVING`/`DISTINCT` is checked against that branch's own `GROUP BY`/select list — a branch that is not itself aligned is refused by name even if another branch is aligned. `LIMIT` has no equivalent relaxation and is always blocked.

### When a model is refused

If a model fails the safety classifier, `smelt run` exits non-zero and prints a diagnostic:

```
Error: Incremental safety check refused the following model(s). Fix the SQL or use
--allow-downgrade to fall back to full-table refresh:
  • Model 'daily_sessions': window functions (OVER clause) are not compatible with
    incremental materialization — they may produce different results on partial data
```

**The recommended fix** is to rewrite the SQL to remove the unsafe pattern, then use `safety_overrides` if you are confident the specific usage is partition-safe.

**Temporary escape hatch** — if you need to unblock a run while the fix is in progress, pass `--allow-downgrade`:

```bash
smelt run --event-time-start 2025-01-01 --event-time-end 2025-01-02 --allow-downgrade
```

With `--allow-downgrade` set, refused models fall back to a full-table refresh for this run. A warning is emitted for each downgraded model. This flag must be passed explicitly every time; it is not persisted anywhere. Using it regularly means the model is not actually running incrementally — fix the SQL instead.

### Declaring monotonicity when smelt can't prove it

Some `event_time`/partition-column expressions cannot be proven monotone by static analysis alone — most commonly a projection through a UDF or another opaque function call. By default smelt rejects the pushdown in that case (the safe, conservative default). If you know the expression is in fact monotone non-decreasing, set `assert_monotonic: true` on the `timeseries:` block:

```yaml
models:
  joined_daily:
    materialization: table
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: partition_key
      partition_column: partition_key
      granularity: day
      assert_monotonic: true
```

```sql
SELECT
    my_normalize_fn(f.partition_key) AS partition_key,
    f.user_id,
    g.attribute
FROM smelt.sources.fact f
JOIN smelt.sources.lookup g ON f.user_id = g.user_id
```

`assert_monotonic` only ever **widens** the pushdown decision — it never overrides a case smelt can already prove is *not* monotone. A constant/`NULL` seed, a row-nondeterministic function (`RANDOM()`, `UUID()`), a periodic or piecewise construct (`MOD`, `CASE`, `EXTRACT`), or an ambiguous/unresolvable column reference is still refused even with the declaration set — only an otherwise-unrecognised function call is admitted.

### Non-deterministic columns

The non-deterministic-function check can be relaxed per column instead of disabled entirely. Declare `columns.<c>.contract: plausible` for the output column and its value is allowed to vary between runs — useful for audit stamps and surrogates such as `inserted_at = NOW()` or `batch_id = UUID()`:

```sql
---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
columns:
  inserted_at:
    contract: plausible
---
SELECT
    event_date,
    user_id,
    COUNT(*) as event_count,
    NOW() as inserted_at
FROM smelt.sources.events
GROUP BY 1, 2
```

The non-deterministic value must flow **directly** into the column carrying `contract: plausible` — a bare `NOW() AS inserted_at` or `RANDOM() AS batch_id` in the SELECT list. Three positions are always rejected, regardless of any declared contract:

- the `event_time_column` or `partition_column` expression,
- any `unique_key` column,
- a row-set-membership or grouping position — `WHERE`, `HAVING`, `JOIN ... ON`, `DISTINCT`, `GROUP BY` keys, or a window's `PARTITION BY`/`ORDER BY`/frame.

Declaring `contract: plausible` on one of these columns is a configuration error, since those columns must stay deterministic no matter what the model opts into.

`NOW()`, `CURRENT_TIMESTAMP`, and `CURRENT_DATE` are frozen once per run, so a direct projection is admitted even when the target column carries no `plausible` contract — pinning already removes the variance the contract exists to gate. `RANDOM()` and `UUID()` values differ row-to-row within the same run, so their target column must still declare it. If the analysis can't confidently attribute a non-deterministic call to a single output column (for example, it's nested inside a CTE or subquery), the model is rejected — use `safety_overrides.allow_nondeterministic` if you've verified the pattern is safe.

### Overriding safety checks

If you understand the implications and the pattern is safe in your specific case, use the top-level `safety_overrides:` key:

```sql
---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
safety_overrides:
  allow_window_functions: true
  allow_having: true
  allow_limit: true
  allow_subqueries: true
  allow_nondeterministic: true
  allow_distinct: true
---
SELECT event_date, user_id, COUNT(*) AS event_count
FROM smelt.sources.events
GROUP BY 1, 2
```

!!! warning
    Only override safety checks when you have verified that your specific query produces correct results on partial data. For example, a window function partitioned by date is safe for daily incremental processing, but one partitioned by user is not.

### Partition-aligned window functions

You do not need a safety override for window functions that are **partition-aligned** — where the `PARTITION BY` keys of the window include the model's `partition_column`. The optimizer admits these directly because each window is evaluated within a single output partition: the DELETE+INSERT contract deletes and re-inserts entire partitions, and a window that cannot look across partition boundaries produces the same result on partial data as on the full table.

For a model with `partition_column: event_date`, these windows are admissible without any override:

```sql
-- Admitted: PARTITION BY contains event_date (equality)
FIRST_VALUE(user_id) OVER (PARTITION BY event_date ORDER BY event_ts) AS first_user

-- Admitted: PARTITION BY contains event_date (superset — extra key is fine)
FIRST_VALUE(user_id) OVER (PARTITION BY event_date, device_id ORDER BY event_ts) AS first_user
```

These windows are **not** admissible (and will be refused) because their `PARTITION BY` keys do not include `event_date`:

```sql
-- Refused: PARTITION BY does not contain event_date
ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_ts) AS rn

-- Refused: no PARTITION BY at all
SUM(amount) OVER (ORDER BY event_ts) AS running_total
```

Use `safety_overrides.allow_window_functions: true` for windows that cannot be partition-aligned and that you have verified are safe in your specific context.

### Bounded-`RANGE` cross-partition windows

A window whose `PARTITION BY` keys do **not** include the model's `partition_column` is still admitted — without an override — when its frame is a bounded `RANGE BETWEEN INTERVAL '…' PRECEDING [AND …]` clause with no `UNBOUNDED` bound. The finite interval is a derivable lookback (or lookforward): the planner widens the *source* read to cover it, so the window is provably partition-local up to that bound even though `PARTITION BY` disagrees with the model's own partition column.

This is what lets a sessionization model window `LAG`/`LEAD` by `device_id` (ordered by event time, framed to a bounded interval) while the model itself is partitioned by a derived `session_start_date`:

```sql
-- Admitted despite PARTITION BY device_id not matching partition_column session_start_date:
-- the bounded 30-minute RANGE frame licenses a 30-minute widened source read.
LAG(event_ts) OVER (
    PARTITION BY device_id ORDER BY event_ts
    RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW
) AS prev_ts
```

The exception is narrow:

- Only a `RANGE` frame qualifies — a `ROWS` or `GROUPS` frame with the same shape is **not** admitted this way (row/group counts do not translate into a time margin the source read can be widened by); a non-aligned `PARTITION BY` with a `ROWS`/`GROUPS` frame is refused as usual.
- An `UNBOUNDED` bound on either side (`UNBOUNDED PRECEDING`, or `UNBOUNDED FOLLOWING`/a `FOLLOWING` bound with no `INTERVAL`) is **not** admitted — an unbounded reach in either direction is cumulative-across-history and forces `PerPartitionOnly` or refusal, the same as a non-partition-aligned window with no frame at all.

## Per-source lookback derivation

For each upstream `smelt.<path>` reference in an incremental model body, the planner derives how far outside the run window that source must be read. This **bound** has the form `(before, after)`: read the source starting `before` seconds before the run start and ending `after` seconds after the run end.

The planner recognises two standard SQL forms:

### Form A — window-frame `RANGE BETWEEN INTERVAL`

When a window function uses an explicit `RANGE BETWEEN INTERVAL '…' PRECEDING` clause, the interval becomes the source's lookback:

```sql
SELECT
    device_id,
    event_ts,
    LAG(event_ts) OVER (
        PARTITION BY device_id
        ORDER BY event_ts
        RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW
    ) AS prev_ts
FROM smelt.silver.events_parsed
```

The planner reads `INTERVAL '30 minutes' PRECEDING` and derives `before = PT30M` for `events_parsed`.

A bare `LAG(x) OVER (PARTITION BY id ORDER BY ts)` without a `RANGE BETWEEN` clause is **not derivable** — the planner cannot determine the lookback and will refuse the model at planning time. Rewrite it with an explicit `RANGE BETWEEN INTERVAL '…' PRECEDING` clause.

**Forward reach (`after`).** The same frame can carry a `FOLLOWING` bound, read as the source's *lookforward* margin — the mirror of the `PRECEDING` case above, from the same walk:

```sql
LEAD(event_ts) OVER (
    PARTITION BY device_id
    ORDER BY event_ts
    RANGE BETWEEN CURRENT ROW AND INTERVAL '2 hours' FOLLOWING
) AS next_ts
```

The planner reads `INTERVAL '2 hours' FOLLOWING` and derives `after = PT2H`. An **unbounded** forward reach — `UNBOUNDED FOLLOWING`, or a `FOLLOWING` bound with no `INTERVAL` literal — is **not derivable**, mirroring the bare-`LAG`/`UNBOUNDED PRECEDING` refusal: the planner cannot bound how far forward the source must be read, so the model is refused at planning time rather than silently treated as zero-margin.

### Form B — explicit WHERE/JOIN interval filters

When the model's WHERE clause or a JOIN condition contains an explicit `INTERVAL` offset on a source column, the interval becomes the source's bound:

```sql
-- Same-column lookback: reads 1 day before the run window
WHERE s.event_date BETWEEN m.partition_date - INTERVAL '1 day' AND m.partition_date
```

```sql
-- Cross-column rebase: UTC timestamps into local-date partitions
WHERE b.event_ts_utc BETWEEN m.event_date_local - INTERVAL '1 day'
                          AND m.event_date_local + INTERVAL '1 day'
```

```sql
-- Forward-only reach: no backward offset, only a lookforward
WHERE e.event_ts BETWEEN m.conversion_ts AND m.conversion_ts + INTERVAL '30 days'
```

Both `BETWEEN` form and paired `>=` / `<` form are read:

```sql
WHERE s.event_date >= m.partition_date - INTERVAL '1 day'
  AND s.event_date < m.partition_date
```

### Viewing derived bounds

Run `smelt explain --json` to see the derived bound map for each incremental model:

```bash
smelt explain --json | jq '.models.sessions.incremental.source_bounds'
```

Example output:
```json
{
  "events_parsed": {
    "type": "bounded",
    "partition_col": "event_date",
    "before": "PT0S",
    "after": "PT0S"
  }
}
```

Durations use ISO-8601 format: `PT30M` (30 minutes), `P1D` (1 day), `PT0S` (zero / partition-local).

### When the bound is not derivable

If the planner cannot derive a bound for a source — a bare window function without a `RANGE` clause, or a computed-expression join with no explicit interval filter — the model is **refused at planning time** with a diagnostic naming the offending source. Rewrite using Form A or Form B to give the planner enough information to prove the lookback.

Sources without `timeseries:` declared (lookup tables, dimension tables) are always read in full and do not appear in the bound map.

### Source-filter pushdown

Once smelt derives the bound for a source, it automatically injects a pushdown `WHERE` filter on that source's `FROM` clause. For a run window `[run_start, run_end)` and a source with bound `Bounded(col, before, after)`:

```sql
-- Injected by the planner on the source's FROM:
WHERE col >= run_start - before
  AND col <  run_end + after
```

This filter is applied **per source reference** before compilation — each `smelt.<path>` reference in the model SQL gets its own pushdown WHERE. The outer model WHERE (constraining the model's output to the run window using the model's own `partition_column`) is unchanged and applied separately.

### Model-to-model chains

A ref to **another incremental model** in the same project is a maintenance-plan edge of the same standing as a source ref. When an incremental model reads another incremental model, the downstream model derives a **creation-trigger cell** for that upstream, clocked by the upstream model's own `timeseries:` declaration — the same clock the upstream maintains itself. Scan bounds compose down the chain exactly as they do for sources: the downstream cell reads the upstream over `[start − before, end + after)`.

`smelt explain <model>` shows these edges. For a model that joins an upstream `silver.events_parsed`, the report lists a creation cell whose trigger names the upstream and whose scan clamp is anchored to the upstream's clock column:

```
Cells (…):
  - group {*} on trigger NewData { source: "silver.events_parsed" }
      corner:    RecomputeRegion
      technique: DeleteInsert
      locality:  partition_local
      scan clamps:
        - source=silver.events_parsed column=event_date before=… after=…
```

If the upstream is a maintained model that declares **no** `timeseries:` clock (and none can be inferred), the edge cannot be clamped. smelt records this as a refusal naming the edge in the plan's `Refusals` section rather than silently dropping the upstream — surfacing that the chain needs a clock to maintain the downstream incrementally. A ref to a `full`-refresh model or a view contributes no creation cell (there is no incremental delta to receive).

**Example.** A sessions model that reads `smelt.silver.events_parsed` (partition column `event_date`) with a derived bound of `PT0S`/`PT0S` (partition-local, no lookback) for a run window `[2024-01-15, 2024-01-16)`:

```sql
-- Before pushdown (model SQL):
WITH sessionized AS (
  SELECT * FROM smelt.functions.sessionize(
    source => smelt.silver.events_parsed, ...
  )
) ...

-- After pushdown (what the engine sees):
WITH sessionized AS (
  SELECT * FROM smelt.functions.sessionize(
    source => (SELECT * FROM smelt.silver.events_parsed
               WHERE event_date >= '2024-01-15'
                 AND event_date < '2024-01-16'), ...
  )
) ...
```

Lookup sources (those without `timeseries:`) are never pushdown candidates — they are read in full each run. Pushdown is per-reference: a self-join on a timeseries source receives the same widened filter on each occurrence.

**Below the outer SELECT.** Pushdown is not limited to a single top-level `FROM` clause. smelt traces whether the model's projected `event_time`/`partition_column` value is a monotone image of a real source's time column — a bare column, a truncation like `DATE_TRUNC`/`time_bucket`, a `CAST` to a temporal type, or a constant `INTERVAL` shift are all traceable; an arithmetic combination of two columns, `CASE`, `COALESCE`, or an unrecognised function are not. When that trace succeeds, the pushdown filter can relocate past constructs that would otherwise keep it stuck at the outer clamp:

`partition_column` does not have to be a timestamp — an ever-increasing integer column (a sequence id, offset, or watermark) is monotone too, and the same trace recognises a constant integer shift over it (`batch_id + 1`, `batch_id - 1`) the same way it recognises a constant `INTERVAL` shift over a timestamp column. A non-monotone integer transform — `batch_id % n` (periodic) or `batch_id * n` (not a constant shift) — is rejected the same way a non-monotone temporal transform is, naming the construct in the diagnostic.

- **`UNION ALL` branches** are traced independently, so a model that appends several timeseries sources still gets a pushdown filter on each source individually.
- **Subquery and CTE bodies** that re-project the partition column get the filter pushed to the real underlying source inside the derived table, not just at the outer query.
- **Joins** resolve which input is the "driving fact" (the one whose column the partition value traces back to) and window only that input — every other joined input (lookup tables, dimension joins) is read in full, so there is no risk of a lookup input being incorrectly time-filtered.

When the trace cannot prove monotonicity for a construct — an ambiguous join input, a non-monotone re-projection — the model falls back to the always-correct outer clamp rather than guessing.

Sources declared in per-entity source YAML files (with a `timeseries:` block — see [Sources guide](sources.md#time-dimension)) are automatically pushdown candidates for every downstream incremental model that reads them. No additional configuration is required on the incremental model.

> **Current scope:** pushdown applies the bound derived from the **outer SQL body**. `smelt.define` function bodies are not yet expanded before bound derivation, so a source whose only INTERVAL pattern is inside a function body receives a partition-local (`PT0S`) filter. The exact run-window filter is still correct for correctness; it may read more data than necessary if the function body introduces a wider lookback. Full expansion-before-derivation is tracked in `docs/plans/20260521-incremental-timeseries-and-derived-bounds.md`.

### Declaring a horizon ceiling (warning only)

The **horizon** — the far edge of the maintained window, past which inputs are no longer read — is always **derived** from the model's own SQL (its lookback, window frames, and join contribution). smelt never trusts a declared horizon for the clamp itself: an under-estimate would silently drop rows that should have been rewritten.

A modeller may still declare a `horizon_ceiling:` as a *warning* ceiling on how far that derived horizon is expected to reach:

```sql
---
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
horizon_ceiling: '30 days'
---
SELECT
    event_date,
    user_id,
    SUM(amount) OVER (
        PARTITION BY user_id
        ORDER BY event_ts
        RANGE BETWEEN INTERVAL '2 hours' PRECEDING AND CURRENT ROW
    ) AS rolling_amount
FROM smelt.events
```

Here the model's own `RANGE BETWEEN INTERVAL '2 hours'` frame derives a 2-hour horizon, comfortably inside the declared 30-day ceiling — no warning. If a future edit widened that frame past 30 days, smelt would emit a compile-time warning naming both the derived reach and the declared ceiling. Either way, **the clamp always uses the derived value** — the ceiling narrows nothing; it only tells you when the model's real reach has grown further than expected.

## Contract relaxations

Every incremental model defaults to the equivalence invariant: its maintained state always equals a full refresh over the same inputs. A model can declare a bounded, checked relaxation of that guarantee with a `contract:` block:

```yaml
contract:
  frozen_horizon: '90 days'      # partition grain only
  deferral: '6 hours'
  cells:                          # optional per-cell refinement, addressed like maintenance.cells
    - columns: [<col>, ...]
      on: <source-address> | backfill
      deferral: '1 day'
```

Two relaxations are available:

- **`frozen_horizon`** — partitions older than `end - frozen_horizon` are never revisited by maintenance. Admitted only on a partition-grain model (`grain: partition`), since a key-grain model has no write-eligibility clamp to narrow. A genuinely late arrival outside the frozen horizon is diagnosed (`ContractLateArrivalOutsideHorizon`), never silently dropped. An unparseable or negative interval, or declaring it on a key-grain model, is `ContractFrozenHorizonInvalid`.
- **`deferral`** — the maintained state may lag its inputs by up to the declared window, licensing a run to be skipped while the pending work stays inside that window, and a later catch-up run to prove it subsumed the skipped work. Admitted on either grain, model-level or per cell, but only where there is a clock to measure lag against — a model-level `deferral` needs a `timeseries:` clock, and a `cells[]` entry's `deferral` needs its `on:` trigger to be a clocked, interval-representable source. A cell whose measured lag exceeds its declared window raises `ContractDeferralExceeded`. An unparseable/negative interval, or a `deferral` with no clock to measure against, is `ContractDeferralInvalid`.

Model-level values are the default for every cell; a `contract.cells[]` entry — addressed the same way as `maintenance.cells[].columns` / `.on` — refines one cell's `deferral`. `frozen_horizon` is model-level only. A per-cell `deferral` refinement validates and prints as declared, but is not yet scheduled — it needs a per-cell maintained frontier the interval ledger does not track yet.

`smelt explain <model>` prints each cell's effective contract — `default` when no `contract:` applies, otherwise the applicable relaxations with their declared intervals:

```
      contract:  default
      contract:  frozen_horizon 90 days, deferral 6 hours (cell)
```

A relaxation is never silent: absent `contract:` is always the default point, and a declared relaxation always shows up here, per cell.

## Batching

When you specify a large time range, smelt automatically chunks it into batches. Each batch is a separate DELETE+INSERT cycle.

The default batch size is determined by the granularity -- for example, daily granularity defaults to processing one day per batch. Use `--batch-size` to override:

```bash
# Process 90 days in 7-day chunks
smelt run --start 2025-01-01 --end 2025-04-01 --batch-size 7
```

Batching provides two benefits:

- **Memory efficiency** -- Each batch processes a bounded amount of data.
- **Progress tracking** -- If a run fails partway through, completed batches are recorded and will not be re-processed.

## Self-referential (ordered) models

A `grain: partition` model may read its own prior partitions — a running balance, a partition-by-partition state machine — by referencing itself (`smelt.<its own path>`) in its SQL. This stays a partitioned `grain: partition` table; it does not become `grain: key`.

Whether a backfill may build its windows in parallel or must build them strictly in temporal order is derived from the model's dependency graph, never declared:

- **No self-edge (the default).** Every window is a pure function of source rows in its own scan range, so a backfill may split into sub-ranges built in any order, including in parallel.
- **A self-edge that provably converges partition-by-partition** (a backward-bounded read of the model's own prior output, e.g. `WHERE bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d`) forces the backfill to build **one partition per batch, strictly in temporal order** — regardless of the model's batch-safety class or any `--batch-size`/`--per-partition` override. A wide, multi-partition batch would read rows the self-reference expects but that have not been written yet.
- **A self-edge that does not provably converge** (a forward read, or an unbounded/whole-history scan) is refused at planning time with a diagnostic naming the non-convergent self-reference — never silently built with the wrong ordering.

A self-referential model still gets the [derived output window](#the-derived-output-window) when its `partition_column` is itself derived and skews away from the driving date column — a run requesting a single day also rewrites the skew-reached neighbouring partitions, exactly as it would for a model with no self-edge. Ordering then applies over the *rebased* range: every partition in it still builds strictly sequentially, one partition per batch. The self-edge's own bounding relation is never read as the skew declaration, even when the self-referenced table's column happens to share the model's own `partition_column` name — only a genuine relation anchored on another source counts. See the [web-analytics tutorial's root-anchored `silver.sessions_chained` table](../examples/web-analytics/ordered-sessions.md) for a real self-referential model whose partition column derives and rebases this way, contrasted directly against a window-independent sibling table built from the same source rule.

A self-referential model's first run needs no manual seeding. Its very first partition can't be created via `CREATE TABLE ... AS SELECT ...` — that statement can't resolve a reference to the table it is in the middle of creating — so when the target table doesn't exist yet, smelt first creates it empty, with the model's inferred output schema, then runs every batch (including the first) as the ordinary partition DELETE+INSERT. The self-read over the empty table sees no prior state, exactly as if the table had been seeded with zero rows by hand.

## Monitoring

### Interval coverage

Use `smelt status` to see which time ranges have been processed:

```bash
# Show all incremental models
smelt status

# Show a specific model
smelt status daily_revenue

# Show gaps in a specific date range
smelt status daily_revenue --since 2025-01-01 --until 2025-03-01
```

### Run history

Use `smelt history` to see past runs:

```bash
# Show recent runs (default: 10)
smelt history

# Show history for a specific model
smelt history daily_revenue

# Show more entries
smelt history --limit 50
```

## Run window vs partition granularity

The `--event-time-start` / `--event-time-end` flags declare a **run window** — not a per-partition invocation. The window must be a positive integer multiple of the model's `timeseries.granularity` aligned to granularity boundaries, but inside that constraint the window size and the partition unit are independent.

For a daily-partitioned model, all of the following are valid:

```bash
# One-day run
smelt run daily_revenue --event-time-start 2025-03-01 --event-time-end 2025-03-02

# One-week run — one engine query, seven partitions written
smelt run daily_revenue --event-time-start 2025-03-01 --event-time-end 2025-03-08

# 60-day backfill — one engine query, sixty partitions written
smelt run daily_revenue --event-time-start 2025-01-01 --event-time-end 2025-03-02
```

For `FullyBatchSafe` models, the entire window runs as a single engine query covering the whole range, then a single `DELETE` over the partitions in the window followed by `INSERT`. Backfilling 60 days is one invocation, not sixty.

For `BoundedSafe(n)` models, the window is auto-chunked into sub-ranges sized to `n` partitions each; each chunk is one engine query and one DELETE+INSERT transaction. `PerPartitionOnly` models still run one partition at a time, sequentially.

Misaligned windows (not an integer multiple of granularity, or endpoints that aren't on granularity boundaries) are rejected at planning time with a clear diagnostic.

## The derived output window

The partitions a run actually writes — the **output window** — are not always the run window verbatim. smelt derives the output window from the run window and the model's own declared time relations:

- **Identity (the common case).** When `partition_column` tracks the event time driving new data (the same column, or a pure truncation of it), the output window equals the run window exactly — nothing changes from the run-window semantics above.
- **A derived, skewing `partition_column`.** Some models compute their partition column from a rule that can place new data in a partition *other than* the one implied by the run window — a session model whose `partition_column` is the session's start date, for example, where a late-night event can extend a session rooted the *previous* day. When such a model declares the relationship (a `WHERE`/`JOIN` filter comparing the driving date column to `partition_column ± INTERVAL '…'`), smelt inverts it: a run over `[start, end)` derives an output window reaching as far as the declared interval allows on either side. A session model capped at one day, for instance, run for a single day `[D, D+1)`, derives the output window `[D-1, D+2)` — the run also rewrites the prior day's partition when new data reaches back into it.

The output window is what the `DELETE` and the output clamp both key off; every written partition's **scan** is sized from the output window's own reach (plus any further lookback the model's SQL declares), never from the narrower run window. Backfill chunking still applies to the derived output window exactly as it does to a plain run window — a wide skew or lookback is split into several bounded sequential updates rather than one large one.

## Rebuilding

`smelt rebuild` rebuilds a model **and all its upstream dependencies** for a given time range. This is useful when you need to reprocess historical data after changing a model's logic.

```bash
smelt rebuild +daily_revenue --start 2025-01-01 --end 2025-02-01
```

The `+` prefix means "include upstream dependencies." smelt will:

1. Walk the dependency graph to find all upstream incremental models.
2. Process each upstream model for the specified time range, in topological order.
3. Process the target model last.

Rebuilding shares the run-window semantics above — one engine query per chunk (or one query for the entire range when models are `FullyBatchSafe`), not per partition.

Rebuilding reprocesses *data* under an unchanged definition. For the complementary problem — migrating a deployed table after the model's *definition* changes, without a full rebuild — see [Migrations](migrations.md).

## Incremental strategies

smelt supports multiple strategies for how data is updated. The strategy is chosen based on your configuration and the backend's capabilities:

| Strategy | When used | Behavior |
|---|---|---|
| `delete_insert` | Default | DELETE matching partitions, then INSERT new data |
| `append` | Append-only workloads | INSERT only, no deletion |
| `insert_overwrite` | Backend-specific optimization | Overwrite entire partitions atomically |

UPSERT (`MERGE`) is **not** a `grain: partition` strategy — it is the backend primitive used by `refresh: incremental` + [`grain: key`](../reference/cumulative-aggregate.md), which is a separate sibling shape with a different equivalence contract. If you want one row per `(unique_key)` collapsed across all source partitions, that's `grain: key`, not `grain: partition`.

## Enrichment joins and dimension updates

A `grain: partition` model that joins a fact source to a dimension table (a `smelt.ref()`/`smelt.sources.*` with no `timeseries:` block) is enrichment: every fact row carries a copy of whatever the dimension held at compute time. When a dimension row changes later — a user renamed, a product recategorized — the fact rows that joined against it are stale until that partition is recomputed.

smelt derives a **maintenance plan** for every `refresh: incremental` model: a matrix of what to do for each group of output columns under each kind of change. For an enrichment join, the columns that came from the dimension form their own group, distinct from the columns that came from the fact table — because a dimension update only ever needs to touch the dimension-derived columns, never the fact-derived ones. Run `smelt explain <model>` to see the derived plan:

```
$ smelt explain daily_events_enriched
model daily_events_enriched  (emits: general (degraded by: join proven OneToMany (row-multiplying) degrades output-delta shape to General), not delta-addressable; grain: partition)

Maintenance plan: daily_events_enriched

Cells (5):
  - group {*} on trigger NewData { source: "raw.events" }
      corner:    RecomputeRegion
      technique: DeleteInsert
      ...
  - group {user_name} on trigger UpstreamMutation { source: "raw.users" }
      corner:    RecomputeRegion
      technique: DeleteInsert
      ...
```

This model's plain inner `JOIN` is exactly the shape that *cannot* be proven row-preserving — its `ON` predicate makes row membership sensitive to the dimension, so no column group of the `SELECT` can be proven value-only — and the `{user_name}` cell falls back to the region `DELETE`+`INSERT` recompute rather than a **column-scoped `MERGE`**. That targeted repair is only available when the join cannot change *which* fact rows exist: a `LEFT JOIN` against a dimension that declares its own `unique_key` proves this (the join can only ever change a value, never admit or drop a fact row), so only the dimension-derived column group repairs column-scoped while everything else the dimension's `ON` predicate might otherwise gate stays untouched. A plain (inner) `JOIN`, or a `LEFT JOIN` against a dimension with no declared `unique_key`, cannot be proven this way — every column group the join touches falls back to the region `DELETE`+`INSERT` recompute instead, since a row-admitting join can change which rows exist, not just their values. Declare the dimension's mutability explicitly on its source YAML so smelt derives a mutation cell at all instead of assuming worst-case immutability:

```yaml
# models/sources/raw/users.yml
mutation_profile:
  kind: mutable_snapshot
```

An unclocked dimension source has no partition column to bound a scan by, so admitting the mutation cell requires accepting a full read of it — name that acceptance on the model that reads it:

```yaml
maintenance:
  scan_bounds:
    per_source:
      raw.users:
        allow_full_scan: true
```

Once the cell is admitted (and the target table already exists), a plain `smelt run` dispatches through the column-scoped `MERGE` on every run over the fact table's own event-time window — no separate "a dimension changed" signal is required, so it re-derives the `MERGE` every time rather than skipping one where nothing upstream moved. To skip runs where the dimension didn't change, declare the landed delta explicitly with [forward propagation](../reference/cli.md#forward-propagation-with---since-upstream) (`smelt run --since-upstream --source raw.users --landed <a>..<b>`), which composes the maintenance plan's per-source scan clamps into a propagation graph and runs only the `(model, region)` pairs that delta can actually affect. If the plan hasn't admitted a cell for your model yet (an unbounded scan, a missing `mutation_profile` declaration, or a backend without column-scoped `MERGE` support), the run falls back to the region-recompute technique (`DELETE`+`INSERT`), which re-reads the dimension's current contents and is always correct, just not column-scoped. `smelt explain` is the way to see today which of your models already have a targeted-write cell derived and ready.

Declaring the dimension's [referential integrity](sources.md#declaring-referential-integrity) narrows the enrichment `MERGE`'s recompute further, down to a point lookup: renaming exactly one user out of a million-row dimension only ever needs to touch the fact rows for that one user, not the fact table's whole event-time window. smelt derives this from two facts together — the enrichment join provably preserves every driving row (licensed by a `LEFT JOIN`, or by a declared `referential_integrity:` on an inner join), and the dimension's own current-vs-previously-seen content diff names exactly which key(s) changed. `smelt explain` reports the row-preservation verdict for the join; the count-preservation check the declaration is paired with re-verifies it against every touched region on each run, and fails the run loudly rather than silently trusting a declaration a data change has since made false.

That content diff is the **fingerprint sidecar**: for a `mutation_profile: mutable_snapshot` source with no native change feed, smelt maintains a per-row content digest alongside the table it reads, and diffs each run's current digests against the previous run's to synthesize exactly which keys changed — the same "which key(s) changed" a real change-data-capture feed would hand you, derived instead of declared. You never configure it directly; it's a byproduct of the mutable-dimension declaration above, refreshed transactionally alongside the write it rides with. If its stored identity stamp ever mismatches what a fresh computation expects — the enrichment projection changed, or the model that consumes it was redefined — that partition's diff degrades to the same whole-table delta an absent sidecar would produce, logged loudly rather than silently trusted. The same sidecar diff also narrows a region `DELETE`+`INSERT` recompute (in place of the column-scoped `MERGE` above, when that technique isn't available) to just the changed dimension rows — DuckDB-only today (`BackendCapabilities::supports_fingerprint_sidecar`); a Spark or BigQuery target keeps the widened-scan recompute for that cell.

### The reconciliation ledger

A **frontier** is the record of which typed deltas a cell has absorbed — smelt's one ledger
concept. Some maintenance cells — the column-scoped `MERGE` above is one — are **additive keyed
folds**: each run applies a source delta on top of the target's existing state rather than
recomputing a region from scratch. To make that safe across retries, backfills, and out-of-order
runs, smelt records, per output region × column group, which source deltas are already reflected
in that region. An already-reflected delta is refused rather than folded a second time (it would
double-count), and recomputing a region — a full `DELETE`+`INSERT` of that region, whether from
`--full-refresh`, a fallback to region-recompute, or an explicit rebuild — resets the region's
ledger entry, since the recompute already incorporates everything up to that point.

This project-wide bookkeeping is the frontier's **reconciliation ledger** realization (see
[State — The reconciliation ledger](../reference/state.md#the-reconciliation-ledger)) — itself
backend-resident (a per-model table in the target backend, folded in the same transaction as the
write it protects), not `.smelt/`-resident. A window-forward keyed model's own `MERGE` writes a
second realization, the **transactional frontier write**, directly into the target table
alongside the row data it accompanies, in the same transaction: it too lives backend-resident,
not in a separate smelt-managed store. You don't declare or configure either realization
directly; `smelt explain <model>` shows whether a given cell routes through the reconciliation
ledger via the `ledger_catch_up` flag on that cell, and prints a recorded `MaintenanceStateDowngraded`
downgrade for any cell whose ideal technique needed a ledger realisation the target backend or
`state.warehouse_tables: none` doesn't supply.

## grain: partition vs grain: key

`grain: partition` produces a partitioned output where each partition's rows survive a `DELETE+INSERT` cycle without changing. `grain: key` collapses partitions into one row per `GROUP BY` key whose value reflects the combined state across history.

- Use `grain: partition` when the answer to "what did this partition produce?" is well-defined and stable.
- Use `grain: key` when the answer is "what's the running total per key?".

A `grain: key` model's identity comes from a top-level `unique_key:` or, absent that, its own `GROUP BY` columns — declaring `grain: key` with neither (no `unique_key:` and no `GROUP BY` in the model's SQL) is a hard error naming the asserted grain and the empty derived key, checked at frontmatter time (no run required).

See the [materializations guide](materializations.md#grain-partition-vs-grain-key) for a side-by-side comparison.

## The composed shape (key + time)

"Partitioned" and "keyed" aren't alternatives you pick between — they're independent facts, and a model may declare both. A `grain: key` model may add a `timeseries:` block to time-partition its output: still one row per key, but that row now lives in a fixed time partition too.

```sql
---
materialization: table
refresh: incremental
unique_key: [event_id]
grain: key
timeseries:
  event_time_column: first_seen_date
  partition_column: first_seen_date
  granularity: day
---
SELECT
    event_id,
    MIN(payload) AS payload,
    MIN(event_date) AS first_seen_date
FROM smelt.sources.raw.events
GROUP BY event_id
```

Adding the clock isn't free — it requires **key temporal locality**: a guarantee that every duplicate delivery of one key stays within a bounded window of itself on the event axis. smelt establishes this one of three ways, in order:

1. **Key-embedded** — the partition column is itself a key column.
2. **Key-determined** — the partition value is a per-key constant, provable from the SQL.
3. **Recurrence-bounded** — a `key_recurrence` window declared on the driving source (`sources.md` §"Source YAML shape"), checked transactionally at merge time rather than trusted.

When none of the three applies, the `timeseries:` block is refused (`KeyedForbidsTimeseries`), naming the missing route.

Establishing locality is worth it: a bare keyed table is a dead end for the rest of the pipeline — nothing downstream can window over it. A composed table is a clocked source in its own right, so a keyed stage can sit inside a propagation chain instead of terminating it, and a key-level change projects downstream instead of an unbounded lookup scan — **exactly**, to the keys' own partitions, under routes 1–2; under route 3 (recurrence-bounded) the projection widens backward by the recurrence bound plus any derived margins, per widen-never-narrow. `smelt explain` reports the established route, the pruned merge target slice, and the derived **settle bound** — how long a written slice can still change.

The [deduplication tutorial page](../examples/web-analytics/deduplication.md) walks this shape end to end: a redelivery-prone event feed deduplicated by a keyed `MIN` fold instead of a `QUALIFY` window, with the recurrence bound doing the work a `safety_overrides` comment would otherwise have to.

## Conditional writes

Some maintenance cells write nothing when a re-run's inputs haven't actually changed. This is an automatic, fail-closed refinement of whichever technique the plan already derived — a column-scoped `MERGE`, a keyed fold `MERGE` (`grain: key`), or their `MERGE`-less staged-candidate equivalents — not a separate strategy you opt into.

### What gets suppressed

- A column-scoped `MERGE`'s matched arm normally does `WHEN MATCHED THEN UPDATE SET *` — every matched row is rewritten, even one whose incoming value is identical to what's already stored. The suppressed form guards the matched arm with an `IS DISTINCT FROM` predicate over the group's own columns instead: `WHEN MATCHED AND (target.c1 IS DISTINCT FROM source.c1 OR …) THEN UPDATE SET *`. A re-run whose delta reproduces the stored state writes zero rows.
- A [keyed fold](#the-composed-shape-key-time) (`grain: key`) compares the stored value against what the fold's own combine expression *would* write, rather than a plain source column, since a keyed fold's matched arm never copies a delta column verbatim. This is how a redelivery-prone feed deduplicated with a `MIN`/`MAX` fold settles to zero writes once a key has seen every duplicate: the combiner is idempotent, so re-merging an already-seen duplicate reproduces the stored value exactly.
- On a backend without `MERGE`, the same no-op elimination is realized as a staged-candidate conditional `DELETE`+`INSERT` inside one transaction: candidates are staged, only the rows whose effect isn't the identity are deleted and re-inserted, and the staging relation is dropped.

On a [composed (key + time)](#the-composed-shape-key-time) output, the suppression compare's target read carries the same locality slice the merge itself uses, so compare cost stays proportional to the slice rather than the whole key space, even at high key cardinality.

### What it costs and what it saves

The saving is write volume: a retried batch, a redelivery storm, or any other idempotent replay writes nothing instead of rewriting every matched row. The cost is one extra `IS DISTINCT FROM` comparison per compared column, evaluated during the merge itself rather than as a separate pass.

### When it's refused

Write suppression is fail-closed over two independent proofs; either one refusing falls back to the plain, always-write matched arm:

- **No proven row identity.** A cell whose row identity is `WholeRow` — no declared `unique_key`, no proven grain key — has no way to address an individual row, so there's no safe way to compare "this row" across runs. `smelt explain <model>` prints this as the cell's `region key:` line (`WholeRow` vs. a named key).
- **An incomparable column.** A column whose value isn't a pure function of the processed inputs — a `columns.<c>.contract: plausible` payload, or a run-pinned clock like `NOW()`/`CURRENT_*` — can legitimately differ from last run's value even though nothing about the inputs changed, so diffing it would either manufacture a false change or mask a real one. One incomparable column in the group refuses suppression for the *whole* cell, not just that column — a partial guard would silently drop that column's real changes.

Refusal never errors — the cell keeps working, falling back to the unconditional matched arm.

### Steering: `prefer` / `technique`

By default, whether a `ColumnScopedMerge`/`KeyedFold` cell's matched arm is suppressed follows a structural rule on top of the two proofs above: a **steady-state** trigger (prior committed state exists to diff against) prefers suppression, while a trigger with nothing to diff against yet — a first build, or a backfill — prefers the plain matched arm, since the compare would be pure overhead there. `maintenance.defaults.prefer`, `maintenance.cells[].prefer`, and `maintenance.cells[].technique` (see [Maintenance Configuration](../reference/smelt-yml.md#maintenance-configuration)) — the same keys that choose among the techniques a cell's plan admits (fold vs. region recompute vs. rederiving columns) — also carry `suppress`/`unconditional` values that steer this orthogonal dimension directly: `prefer: suppress` / `prefer: unconditional` nudge the default without ever refusing, and `technique: suppress` / `technique: unconditional` force it — a hard pin never bypasses the row-identity/column-comparability proof above, so pinning `suppress` over a cell that proof itself refuses is a fail-loud error, not a silent downgrade. `smelt explain` prints the resolved variant and why (`preference`, `first-build posture`, or `default` when the proof itself never admitted suppression) alongside each cell.

This preference/pin ladder only reaches the `ColumnScopedMerge` live run path today. A `KeyedFold` cell under `refresh: keyed` still always honours a `Suppressed` verdict unconditionally at run time, regardless of trigger or any `prefer`/`technique` override — `smelt explain` resolves and prints the ladder's answer for a `KeyedFold` cell too, but that printed answer doesn't yet drive the live keyed-fold executor.

### Known limitation: `smelt explain` doesn't show which cells suppress

`smelt explain <model> --show-sql` renders every `ColumnScopedMerge`/`KeyedFold` cell's unconditional matched arm today, even for a cell that resolves to suppressed at run time — the reporting path hasn't been wired to the same check the live run already applies. The `region key:` line is still a reliable signal for the row-identity half of the rule (`WholeRow` means that cell never suppresses); there is no equivalent surface yet for the column-comparability half.

## Observed deltas and no-op cascades

A suppressed write (above) still has to decide, *downstream*, which regions actually need recomputing — a re-run that wrote nothing changed nothing for its own consumers either. smelt records this directly: when a column-scoped `MERGE` cell runs, it records the changed-row set — restricted to the columns whose flutter is actually comparable, the same proof write suppression itself relies on — in the same backend transaction as the write. A run that suppressed every row records a **present-but-empty** delta, distinct from no record at all; a run that never went through a recording cell (or predates this feature) leaves the delta **absent**. This asymmetry matters to every consumer: an empty delta means "ran, changed nothing"; an absent delta means "assume the worst" and falls back to the full written window (widen-never-narrow — a missing record never narrows what a downstream run treats as dirty).

On a [composed (key + time)](#the-composed-shape-key-time) output, a recorded delta is more than a yes/no — smelt projects it onto the model's own partition axis so forward propagation (`smelt run --since-upstream`) can dirty exactly the touched partitions instead of the whole written window:

- **Routes 1–2 (key-embedded, key-determined)** project **exactly**: under these routes a stored row's partition value is a per-key constant, so the changed keys' own partitions are the whole story.
- **Route 3 (recurrence-bounded)** widens the projection backward by the recurrence bound plus the route's own margins, since a key's partition value can move under this route (a later, out-of-order row can supersede an earlier partition assignment).

Chain a few of these stages together and the payoff compounds: a stable upstream that keeps suppressing its own writes hands its consumers an empty delta, which the graph turns into zero scheduled work, run after run — a **no-op cascade** with a provable horizon behind it, not a heuristic short-circuit. This composes with each composed output's derived **settle bound** ([above](#the-composed-shape-key-time)): a downstream consumer can skip a settled slice unconditionally (it provably can't change again) and skip an unsettled slice whose recorded delta happens to be empty (nothing changed *this* run, even though it still could next run). Together, a quiet upstream degenerates the whole chain to no-op propagation, and the settle bound bounds how long that quiet has to hold before it's provably permanent.

`smelt explain <model>` surfaces both halves directly. Each `ColumnScopedMerge` cell — the only technique family recording is wired for today; `KeyedFold` and the staged-candidate write family do not record yet, so their cells print no such line at all — prints an `observed-delta recording:` line. That line reads `yes` only when the cell's matched arm actually suppresses (the same two proofs above: a proven per-row identity, `region key: Key(...)`, over columns all proven comparable across runs); a `WholeRow`-identity cell, or one with an incomparable compared column, prints `no` — there's nothing to record when the write always rewrites every matched row unconditionally. A composed model's `Key temporal locality:` block prints an `observed-delta projection:` line alongside its route and settle bound: `exact (key-embedded)` / `exact (key-determined)` for routes 1–2, `` widened by `r` + margins `` for route 3. A bare keyed model (no established locality) prints no projection line at all — there's no partition axis to project onto.

Recording and projection are both static facts about the derived plan — `smelt explain` never opens a backend connection, so it reports what a cell's technique *would* record and how its route *would* project, not what a specific past run actually recorded. Reading the live recorded delta for a specific run is `smelt run --since-upstream`'s job, not `explain`'s.

**What a recorded delta narrows.** `smelt run --since-upstream --source <model-address> --landed <a>..<b>` reads the recorded delta for that exact `(model, window)` before propagating: absent, it falls back to the declared `--landed` window unchanged; present-and-empty, it propagates nothing downstream; present-and-non-empty, it narrows to the recorded partitions (projected per the routes above). This read is DuckDB-scoped today, matching the write side — a non-DuckDB target reads back "absent" and falls back rather than erroring. `smelt build <model> --period <a>..<b> --include-upstreams` (backward resolution) never consults a recorded delta at all — it answers "what must exist over this period", a question a change record can't soundly narrow.

**Which writes record a delta.** Three write families record their observed output delta in the same backend transaction as the write: the column-scoped conditional `MERGE`, the change-suppressed keyed fold, and the staged-candidate conditional recompute (whose recorded delta also includes any departed key — present before the run, absent from the candidate set). An unconditional write of any family never records one — the record is a byproduct of a conditional write's own change detection, never derived after the fact. A non-DuckDB backend refuses fail-loud on a write that would otherwise record a delta, rather than silently writing without recording one.

**Settled vs. unsettled empty.** A present-and-empty recorded delta composes with the model's own derived settle bound (the key temporal locality section above): once the delta's window has passed the settle bound, no later-arriving row can still touch it, so the dirty-set report names it a **settled no-op** rather than merely empty-this-run. This is a reporting distinction only — both cases already propagate zero dirt; the settle bound never prunes further work beyond what an empty delta already prunes.

## Repairing only the affected groups

A non-invertible combiner (`MAX`, `MAX_BY`, and similar overwrite-family folds) can't undo a retracted contribution — when its input changes underneath it, the plan normally refuses to fold at all and falls back to a full refresh. The repair family narrows that refusal for the common case where the change is a **retraction or mutation over a mutable dimension**: if the affected output keys are provably finite, smelt recomputes only those groups from their bounded input slice instead of rebuilding the whole table.

```sql
---
materialization: table
refresh: incremental
grain: key
unique_key: customer_id
---
SELECT customer_id, MAX(order_total) AS max_order
FROM smelt.sources.orders
WHERE order_date BETWEEN TIMESTAMP '2026-01-01' - INTERVAL '3 days' AND TIMESTAMP '2026-01-01'
GROUP BY customer_id
```

If `orders` is declared `mutation_profile: mutable_snapshot` (a table whose rows can be updated or deleted in place), a correction to an existing order — or its deletion — narrows to a per-group recompute over just the affected `customer_id`s, rather than refusing outright.

`smelt explain <model>` prints the repair cell's own stanza — the affected-key slice (labelled a sound over-approximation: it may include more keys than were truly touched, never fewer), the bounded per-group read slice, and how the affected keys were discovered:

```
  - group {max_order} on trigger NewData { source: "orders" }
      corner:    ColumnMerge
      technique: PerGroupRecompute
      ...
      repair key slice: customer_id (sound over-approximation)
      repair read bound: source=orders column=order_date before=... after=...
      affected-key discovery: group-grain fingerprint-sidecar diff (mutable_snapshot, obligation 7)
```

For an append-only source (no native deletion), the discovery mechanism is the ordinary clamped current-source scan instead of the sidecar diff — a `mutable_snapshot` source needs the sidecar because a key whose entire window contribution was deleted leaves no row for a scan to witness.

### Pinning `write: diff_patch`

By default a repair cell writes its recomputed groups with a targeted `DELETE`+`INSERT`. Pinning `write: diff_patch` instead computes the candidate slice, diffs it against stored state, and writes only the difference:

```yaml
maintenance:
  cells:
  - on: orders
    columns: [max_order]
    write: diff_patch
```

`smelt explain` reports the resolved write mechanism and its delete-leg verdict alongside the repair stanza:

```
      write mechanism: diff_patch
      diff_patch delete leg: complete
```

A `PerGroupRecompute` cell's own key-temporal-locality premise (the same bounded slice that discharges the repair family's admission) is what makes the delete leg **complete** — the diff can safely delete a stored key absent from the recomputed slice, because that slice provably contains every row that could contribute to any key in it.

## Schema evolution

When an incremental model's output schema changes (columns added, types widened, struct fields modified), smelt can automatically migrate the existing table instead of rebuilding it from scratch. See [Schema Evolution](schema-evolution.md) for full details on:

- Safe vs unsafe changes and how each is handled
- Complex type support (structs, arrays, maps)
- Backend-specific behavior (DuckDB, Spark+Delta, Spark+Parquet)
- The `--allow-full-refresh` flag for changes that require a full table rebuild

## Further reading

- [Materializations](materializations.md) for an overview of all materialization types
- [grain: key](../reference/cumulative-aggregate.md) for key-grain running state (one row per key)
- [Deduplication without the workaround](../examples/web-analytics/deduplication.md) for the composed key + time shape, worked end to end
- [Model Selection](model-selection.md) for running specific models with `--select`
- [Schema Evolution](schema-evolution.md) for automatic schema migration during incremental runs
