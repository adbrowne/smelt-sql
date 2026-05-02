# Incremental Models

Incremental materialization lets you process only new or changed data instead of rebuilding an entire table from scratch. For large datasets, this can reduce run times from hours to seconds.

## How it works

smelt uses a **DELETE+INSERT** strategy on time partitions. For each incremental run:

1. **DELETE** rows in the output table where the partition column falls within the requested time range.
2. **Run the query** with a WHERE filter restricting source data to that time range.
3. **INSERT** the results into the output table.

This approach is idempotent -- running the same time range twice produces the same result, because the DELETE step clears any previous output before inserting.

## Configuration

Incremental behavior is configured either in YAML frontmatter within the SQL file or in the `models` section of `smelt.yml`.

### Frontmatter example

```sql
---
materialization: table
incremental:
  enabled: true
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

### smelt.yml example

```yaml
models:
  daily_revenue:
    materialization: table
    incremental:
      enabled: true
      event_time_column: transaction_timestamp
      partition_column: revenue_date
      granularity: day
```

### Configuration fields

| Field | Required | Description |
|---|---|---|
| `enabled` | No | Defaults to `true`. Set to `false` to disable incremental processing. |
| `event_time_column` | Yes | Column in the **source** data used for the WHERE filter. smelt injects `WHERE event_time_column >= start AND event_time_column < end`. |
| `partition_column` | Yes | Column in the **output** data used for the DELETE step. smelt runs `DELETE FROM table WHERE partition_column >= start AND partition_column < end` before inserting. |
| `granularity` | Yes | Time granularity for partitions. One of: `hour`, `day`, `week`, `month`, `quarter`, `year`. |
| `unique_key` | No | List of columns that uniquely identify a row. When present, the backend may choose a MERGE strategy instead of DELETE+INSERT. |
| `safety_overrides` | No | Override safety checks for patterns that may behave differently on partial data. See [Safety analysis](#safety-analysis). |

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
    incremental:
      enabled: true
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
| HAVING with aggregates | `HAVING COUNT(*) > 10` may filter out groups that would pass if all data were present. |
| LIMIT | `LIMIT 100` on partial data returns different rows than on the full table. |
| Non-deterministic functions | `RANDOM()`, `NOW()`, and similar functions produce different results on each run. |
| Subqueries | Subqueries may reference data outside the filtered time range. |
| DISTINCT | `SELECT DISTINCT` on partial data may miss duplicates that span partition boundaries. |

### Overriding safety checks

If you understand the implications and the pattern is safe in your specific case, use `safety_overrides`:

```yaml
models:
  my_model:
    materialization: table
    incremental:
      enabled: true
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
```

!!! warning
    Only override safety checks when you have verified that your specific query produces correct results on partial data. For example, a window function partitioned by date is safe for daily incremental processing, but one partitioned by user is not.

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
