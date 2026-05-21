# Incremental Models

Incremental materialization lets you process only new or changed data instead of rebuilding an entire table from scratch. For large datasets, this can reduce run times from hours to seconds.

## How it works

smelt uses a **DELETE+INSERT** strategy on time partitions. For each incremental run:

1. **DELETE** rows in the output table where the partition column falls within the requested time range.
2. **Run the query** with a WHERE filter restricting source data to that time range.
3. **INSERT** the results into the output table.

This approach is idempotent -- running the same time range twice produces the same result, because the DELETE step clears any previous output before inserting.

## Configuration

Incremental behavior is configured using two frontmatter blocks:

- **`timeseries:`** declares the time dimension — which column is the event time, which column partitions the output, and at what granularity. See the [timeseries reference](../reference/timeseries.md) for the full key table.
- **`incremental:`** opts the model into incremental execution and carries strategy-specific keys.

Both blocks are required when running incrementally. Declaring `incremental:` without `timeseries:` is a validation error (`TimeseriesRequiredForIncremental`).

### Frontmatter example

```sql
---
materialization: table
timeseries:
  event_time_column: transaction_timestamp
  partition_column: revenue_date
  granularity: day
incremental:
  enabled: true
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

`timeseries:` declares the time dimension; `incremental:` opts the model into incremental execution.

### smelt.yml example

```yaml
models:
  daily_revenue:
    materialization: table
    timeseries:
      event_time_column: transaction_timestamp
      partition_column: revenue_date
      granularity: day
    incremental:
      enabled: true
```

### `incremental:` fields

| Field | Required | Description |
|---|---|---|
| `enabled` | No | Defaults to `true`. Set to `false` to disable incremental processing. |
| `unique_key` | No | List of columns that uniquely identify a row. When present, the backend may choose a MERGE strategy instead of DELETE+INSERT. |
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
    timeseries:
      event_time_column: transaction_timestamp
      partition_column: revenue_date
      granularity: day
    incremental:
      enabled: true
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
| HAVING with aggregates | `HAVING COUNT(*) > 10` may filter out groups that would pass if all data were present. |
| LIMIT | `LIMIT 100` on partial data returns different rows than on the full table. |
| Non-deterministic functions | `RANDOM()`, `NOW()`, and similar functions produce different results on each run. |
| Subqueries | Subqueries may reference data outside the filtered time range. |
| DISTINCT | `SELECT DISTINCT` on partial data may miss duplicates that span partition boundaries. |

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

### Overriding safety checks

If you understand the implications and the pattern is safe in your specific case, use `safety_overrides`:

```yaml
models:
  my_model:
    materialization: table
    timeseries:
      event_time_column: event_time
      partition_column: event_date
      granularity: day
    incremental:
      enabled: true
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

## Backbuilding

Backbuilding rebuilds a model **and all its upstream dependencies** for a given time range. This is useful when you need to reprocess historical data after changing a model's logic.

```bash
smelt backbuild +daily_revenue --start 2025-01-01 --end 2025-02-01
```

The `+` prefix means "include upstream dependencies." smelt will:

1. Walk the dependency graph to find all upstream incremental models.
2. Process each upstream model for the specified time range, in topological order.
3. Process the target model last.

!!! tip
    Backbuilding respects the same `--batch-size` and `--per-partition` options as `smelt run`.

## Incremental strategies

smelt supports multiple strategies for how data is updated. The strategy is chosen based on your configuration and the backend's capabilities:

| Strategy | When used | Behavior |
|---|---|---|
| `delete_insert` | Default when no `unique_key` | DELETE matching partitions, then INSERT new data |
| `merge` | When `unique_key` is set and backend supports it | MERGE/UPSERT based on unique key columns |
| `append` | Append-only workloads | INSERT only, no deletion |
| `insert_overwrite` | Backend-specific optimization | Overwrite entire partitions atomically |

## Schema evolution

When an incremental model's output schema changes (columns added, types widened, struct fields modified), smelt can automatically migrate the existing table instead of rebuilding it from scratch. See [Schema Evolution](schema-evolution.md) for full details on:

- Safe vs unsafe changes and how each is handled
- Complex type support (structs, arrays, maps)
- Backend-specific behavior (DuckDB, Spark+Delta, Spark+Parquet)
- The `--allow-full-refresh` flag for changes that require a full table rebuild

## Further reading

- [Materializations](materializations.md) for an overview of all materialization types
- [Model Selection](model-selection.md) for running specific models with `--select`
- [Schema Evolution](schema-evolution.md) for automatic schema migration during incremental runs
