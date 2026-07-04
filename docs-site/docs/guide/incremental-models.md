# Incremental Models

Incremental materialization lets you process only new or changed data instead of rebuilding an entire table from scratch. For large datasets, this can reduce run times from hours to seconds.

## How it works

smelt uses a **DELETE+INSERT** strategy on time partitions. For each incremental run:

1. **DELETE** rows in the output table where the partition column falls within the requested time range.
2. **Run the query** with a WHERE filter restricting source data to that time range.
3. **INSERT** the results into the output table.

This approach is idempotent -- running the same time range twice produces the same result, because the DELETE step clears any previous output before inserting.

## Configuration

Incremental behavior is configured by selecting `refresh: batched`, plus one required frontmatter block:

- **`refresh: batched`** opts the model into incremental (batched) execution. It implies a stored `table` — you do not also declare `materialization: table`.
- **`timeseries:`** declares the time dimension — which column is the event time, which column partitions the output, and at what granularity. See the [timeseries reference](../reference/timeseries.md) for the full key table.
- **`batched:`** (optional) carries strategy-specific keys (`unique_key`, `safety_overrides`).

`timeseries:` is required when `refresh: batched` is set. Declaring `refresh: batched` without `timeseries:` is a validation error (`TimeseriesRequiredForBatched`). A `batched:` block without `refresh: batched` is also a validation error.

### Frontmatter example

```sql
---
materialization: table
refresh: batched
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

`refresh: batched` opts the model into incremental execution; `timeseries:` declares the time dimension.

### smelt.yml example

```yaml
models:
  daily_revenue:
    materialization: table
    refresh: batched
    timeseries:
      event_time_column: transaction_timestamp
      partition_column: revenue_date
      granularity: day
```

### `batched:` fields

| Field | Required | Description |
|---|---|---|
| `unique_key` | No | List of columns that uniquely identify a row. When present, the backend may choose a MERGE strategy instead of DELETE+INSERT. |
| `nondeterministic_columns` | No | Output columns exempt from the determinism requirement (e.g. an `inserted_at = NOW()` audit stamp). See [Non-deterministic columns](#non-deterministic-columns). |
| `safety_overrides` | No | Override safety checks for patterns that may behave differently on partial data. See [Safety analysis](#safety-analysis). |

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

## Running incremental models

### Explicit time range

Specify the start (inclusive) and end (exclusive) dates:

```bash
smelt run --start 2025-01-01 --end 2025-01-08
```

The longer form `--event-time-start` and `--event-time-end` is also supported and behaves identically.

!!! note
    Non-incremental models in the DAG are still executed normally. The time range only affects models with incremental configuration.

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
    refresh: batched
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
    refresh: batched
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

The non-deterministic-function check can be relaxed per column instead of disabled entirely. List the output column in `batched.nondeterministic_columns` and the value is allowed to vary between runs — useful for audit stamps and surrogates such as `inserted_at = NOW()` or `batch_id = UUID()`:

```yaml
models:
  audit_stamped:
    materialization: table
    refresh: batched
    timeseries:
      event_time_column: event_time
      partition_column: event_date
      granularity: day
    batched:
      nondeterministic_columns:
        - inserted_at
```

```sql
SELECT
    event_date,
    user_id,
    COUNT(*) as event_count,
    NOW() as inserted_at
FROM smelt.sources.events
GROUP BY 1, 2
```

The non-deterministic value must flow **directly** into a listed column — a bare `NOW() AS inserted_at` or `RANDOM() AS batch_id` in the SELECT list. Three positions are always rejected, regardless of the list:

- the `event_time_column` or `partition_column` expression,
- any `unique_key` column,
- a row-set-membership or grouping position — `WHERE`, `HAVING`, `JOIN ... ON`, `DISTINCT`, `GROUP BY` keys, or a window's `PARTITION BY`/`ORDER BY`/frame.

Listing one of these columns in `nondeterministic_columns` is a configuration error, since those columns must stay deterministic no matter what the model opts into.

`NOW()`, `CURRENT_TIMESTAMP`, and `CURRENT_DATE` are frozen once per run, so a direct projection is admitted even when the target column is **not** listed — pinning already removes the variance the list exists to gate. `RANDOM()` and `UUID()` values differ row-to-row within the same run, so their target column must still be listed. If the analysis can't confidently attribute a non-deterministic call to a single output column (for example, it's nested inside a CTE or subquery), the model is rejected — use `safety_overrides.allow_nondeterministic` if you've verified the pattern is safe.

### Overriding safety checks

If you understand the implications and the pattern is safe in your specific case, use `safety_overrides`:

```yaml
models:
  my_model:
    materialization: table
    refresh: batched
    timeseries:
      event_time_column: event_time
      partition_column: event_date
      granularity: day
    batched:
      safety_overrides:
        allow_window_functions: true
        allow_having: true
        allow_limit: true
        allow_subqueries: true
        allow_nondeterministic: true
        allow_distinct: true
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

- **`UNION ALL` branches** are traced independently, so a model that appends several timeseries sources still gets a pushdown filter on each source individually.
- **Subquery and CTE bodies** that re-project the partition column get the filter pushed to the real underlying source inside the derived table, not just at the outer query.
- **Joins** resolve which input is the "driving fact" (the one whose column the partition value traces back to) and window only that input — every other joined input (lookup tables, dimension joins) is read in full, so there is no risk of a lookup input being incorrectly time-filtered.

When the trace cannot prove monotonicity for a construct — an ambiguous join input, a non-monotone re-projection — the model falls back to the always-correct outer clamp rather than guessing.

Sources declared in per-entity source YAML files (with a `timeseries:` block — see [Sources guide](sources.md#time-dimension)) are automatically pushdown candidates for every downstream incremental model that reads them. No additional configuration is required on the incremental model.

> **Current scope:** pushdown applies the bound derived from the **outer SQL body**. `smelt.define` function bodies are not yet expanded before bound derivation, so a source whose only INTERVAL pattern is inside a function body receives a partition-local (`PT0S`) filter. The exact run-window filter is still correct for correctness; it may read more data than necessary if the function body introduces a wider lookback. Full expansion-before-derivation is tracked in `docs/plans/20260521-incremental-timeseries-and-derived-bounds.md`.

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

## Backbuilding

Backbuilding rebuilds a model **and all its upstream dependencies** for a given time range. This is useful when you need to reprocess historical data after changing a model's logic.

```bash
smelt backbuild +daily_revenue --start 2025-01-01 --end 2025-02-01
```

The `+` prefix means "include upstream dependencies." smelt will:

1. Walk the dependency graph to find all upstream incremental models.
2. Process each upstream model for the specified time range, in topological order.
3. Process the target model last.

Backbuilding shares the run-window semantics above — one engine query per chunk (or one query for the entire range when models are `FullyBatchSafe`), not per partition.

## Incremental strategies

smelt supports multiple strategies for how data is updated. The strategy is chosen based on your configuration and the backend's capabilities:

| Strategy | When used | Behavior |
|---|---|---|
| `delete_insert` | Default | DELETE matching partitions, then INSERT new data |
| `append` | Append-only workloads | INSERT only, no deletion |
| `insert_overwrite` | Backend-specific optimization | Overwrite entire partitions atomically |

UPSERT (`MERGE`) is **not** an incremental strategy — it is the backend primitive used by the [`cumulative_aggregate` materialization](materializations.md#cumulative_aggregate), which is a separate sibling rule with a different equivalence contract. If you want one row per `(unique_key)` collapsed across all source partitions, that's `cumulative_aggregate`, not `incremental`.

## Incremental vs cumulative

`incremental` produces a partitioned output where each partition's rows survive a `DELETE+INSERT` cycle without changing. `cumulative_aggregate` collapses partitions into one row per `GROUP BY` key whose value reflects the combined state across history.

- Use `incremental` when the answer to "what did this partition produce?" is well-defined and stable.
- Use `cumulative_aggregate` when the answer is "what's the running total per key?".

See the [materializations guide](materializations.md#incremental-vs-cumulative_aggregate) for a side-by-side comparison.

## Schema evolution

When an incremental model's output schema changes (columns added, types widened, struct fields modified), smelt can automatically migrate the existing table instead of rebuilding it from scratch. See [Schema Evolution](schema-evolution.md) for full details on:

- Safe vs unsafe changes and how each is handled
- Complex type support (structs, arrays, maps)
- Backend-specific behavior (DuckDB, Spark+Delta, Spark+Parquet)
- The `--allow-full-refresh` flag for changes that require a full table rebuild

## Further reading

- [Materializations](materializations.md) for an overview of all materialization types
- [cumulative_aggregate](materializations.md#cumulative_aggregate) for cumulative state (one row per key)
- [Model Selection](model-selection.md) for running specific models with `--select`
- [Schema Evolution](schema-evolution.md) for automatic schema migration during incremental runs
