# Materializations

A materialization controls how smelt persists the results of a model in the target database. There are five materialization types, each suited to different use cases.

## Materialization types

### view (default)

Creates a SQL view. No data is stored -- the query is re-evaluated each time the view is read.

```sql
CREATE VIEW user_events AS
  SELECT user_id, COUNT(*) as event_count
  FROM events GROUP BY 1;
```

Best for:

- Lightweight transforms and staging layers
- Models that are queried infrequently
- Keeping the database small during development

### table

Creates a physical table. Data is persisted and only recomputed when you run the model again.

```sql
CREATE TABLE daily_revenue AS
  SELECT date, SUM(amount) as revenue
  FROM transactions GROUP BY 1;
```

Best for:

- Frequently queried models
- Heavy aggregations you don't want to recompute on every read
- Models with many downstream dependents
- Incremental models (incremental requires `table` materialization)
- Cumulative aggregates (`refresh: cumulative` requires `table` materialization)

### ephemeral

Not materialized at all. The model's SQL is inlined as a CTE (Common Table Expression) into every downstream model that references it.

Best for:

- Intermediate transformation steps that don't need to be queried directly
- Reducing the number of objects in your database
- Simple column renames or type casts

!!! warning
    Ephemeral models cannot have incremental configuration, `refresh: cumulative`, or target overrides. smelt will raise an error if you try to combine these.

### materialized_view

Creates a backend-managed materialized view. The database handles refresh scheduling and invalidation.

Best for:

- Backends that support materialized view refresh (PostgreSQL, Databricks)
- Cases where you want the database to manage the refresh lifecycle

!!! note
    Materialized views are refreshed atomically by the backend. Incremental configuration on a materialized view has no effect and will produce a warning.

### cumulative_aggregate

Stateful merge into one row per `GROUP BY` key. Each daily run only aggregates the new partition's events and merges them into the running cumulative state. The unique key, the per-column aggregator, and the cross-partition combiner are all derived from the SQL.

Use `materialization: table` together with `refresh: cumulative` to enable this mode:

```sql
---
materialization: table
refresh: cumulative
---
SELECT
    device_id,
    user_id,
    COUNT(*)      AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
```

The output has one row per `(device_id, user_id)`. There is no `event_date` column — partitions collapse into a per-key row. The driving partition shape is read from the source's `timeseries:` declaration; running with `--event-time-start D --event-time-end D+N` merges the N partitions in temporal order. Without a run window, the model falls back to a single-shot full refresh.

Each non-key projection must be a direct call to one of the **allowlisted aggregators**, which are paired with a fixed cross-partition combiner:

| Per-partition aggregator | Cross-partition combiner |
|---|---|
| `COUNT(...)` | `SUM` |
| `SUM(...)` | `SUM` |
| `MIN(...)` | `MIN` |
| `MAX(...)` | `MAX` |
| `BOOL_AND(...)` | `BOOL_AND` |
| `BOOL_OR(...)` | `BOOL_OR` |
| `BIT_AND(...)` | `BIT_AND` |
| `BIT_OR(...)` | `BIT_OR` |
| `BIT_XOR(...)` | `xor()` |

Anything outside the allowlist — `STRING_AGG`, `LIST_AGG`, `FIRST`, `LAST`, `AVG`, composite expressions like `SUM(x) + 1` — is rejected at planning time. Each allowed aggregator is commutative and associative, which is the property that lets the rule merge partitions in any order and still produce the same final state.

Best for:

- Cumulative counts and identity edge sets (e.g. `(device, user)` co-occurrence)
- Per-key rollups where each daily run should be cheap — proportional to that day's source rows, not the full source history
- Tables consumed downstream as a lookup (no `partition_column` on the output)

!!! warning "Forbidden combinations"
    Cumulative models cannot declare a `timeseries:` block (the output has no partition column — the partition shape comes from the source) and cannot declare a `batched:` block (the two are sibling rules with different equivalence contracts). Combining them produces a `CumulativeForbidsTimeseries` or `CumulativeForbidsBatched` error. Using `refresh: cumulative` on an `ephemeral` model is also a hard error.

!!! note "Reprocessing"
    v1 does not support per-partition reprocessing for already-merged source partitions. If a past partition's data changes, run with `--full-refresh` to truncate and rebuild from scratch.

For the deeper rationale, see the [cumulative_aggregate spec](../reference/cumulative-aggregate.md).

!!! note "Tests are not a materialization"
    A unit test is a `smelt.test` declaration, not a `materialization` value — it lives on the kind axis alongside models and functions, produces no database object, and is run by `smelt test` (never by `smelt run`). See the [Testing guide](testing.md) for the `smelt.test` grammar (`PASSING`/`EXPECT`, the `#cte` operator, `check_order`/`cases`).

## Setting materialization

There are three ways to set a model's materialization. They are listed here in order of precedence (highest to lowest):

### 1. YAML frontmatter in the SQL file

```sql
---
materialization: table
---

SELECT user_id, SUM(amount) as lifetime_value
FROM smelt.transactions
GROUP BY 1
```

### 2. models section in smelt.yml

```yaml
models:
  daily_revenue:
    materialization: table
  user_activity:
    materialization: table
  staging_users:
    materialization: ephemeral
```

### 3. default_materialization in smelt.yml

```yaml
default_materialization: view
```

When `materialization` is omitted in the SQL frontmatter, in `models.<name>` of `smelt.yml`, **and** at the project-level `default_materialization` key, a model is materialized as a `view` — that is the built-in fallback. See [`smelt.yml` reference](../reference/smelt-yml.md#materialization-types) for the precedence chain.

!!! tip
    A common pattern is to set `default_materialization: view` in `smelt.yml`, then override specific models to `table` where performance matters. This keeps development fast while ensuring production-critical models are materialized.

## Refresh axis

The `refresh:` frontmatter key controls how a stored model's output is recomputed on each run. It applies only to `materialization: table` and `materialization: materialized_view`; setting it on other materialization types has no effect for `view` (a warning is emitted) and is a hard error for `ephemeral`.

| Value | Meaning |
|---|---|
| `full` (default) | Rebuild the table from scratch on every run. |
| `cumulative` | Cumulative-aggregate merge: one row per `GROUP BY` key, grown across partitions. |

When `refresh:` is omitted, `full` is assumed — the model always rebuilds completely.

### refresh: full (default)

No frontmatter key needed. The model always rebuilds from scratch:

```sql
---
materialization: table
---
SELECT date, SUM(amount) AS revenue
FROM transactions
GROUP BY 1
```

### refresh: cumulative

Enables the cumulative-aggregate merge loop. The model accumulates one row per `GROUP BY` key across all processed partitions — see [cumulative_aggregate](#cumulative_aggregate) above for the full semantics, allowed aggregators, and constraint rules.

```sql
---
materialization: table
refresh: cumulative
---
SELECT device_id, user_id, COUNT(*) AS event_count
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
```

## Decision guide

| Scenario | Recommended |
|---|---|
| Staging layer / light transforms | `view` |
| Aggregations queried by BI tools | `table` |
| Intermediate step used by one downstream | `ephemeral` |
| Model with many downstream dependents | `table` |
| Incremental processing (per-partition output) | `incremental` (per-partition `timeseries:` output) |
| Cumulative state (one row per key across history) | `table` + `refresh: cumulative` |
| Database-managed refresh | `materialized_view` |
| Development / iteration | `view` |

## Incremental vs cumulative (refresh: cumulative)

Both shapes are time-aware, but they uphold different contracts:

| Property | `incremental` | `refresh: cumulative` |
|---|---|---|
| Frontmatter | `materialization: table` + `timeseries:` + `incremental:` | `materialization: table` + `refresh: cumulative` |
| Output shape | One row per `(partition_column, …)` — partitioned | One row per `GROUP BY` key — collapsed |
| Declares `timeseries:`? | Yes (the model's output is a timeseries) | No (forbidden; reads partition shape from source) |
| Equivalence contract | Per-partition equivalence with full refresh | Cross-partition equivalence under any partition ordering |
| Re-running a past partition | Idempotent (DELETE+INSERT) | Refused in v1; use `--full-refresh` |
| Backend primitive | `DELETE` + `INSERT` per partition | `MERGE INTO` with per-column combiners |

If the question is "what's the day's contribution?", use `incremental`. If the question is "what's the running total per key?", use `refresh: cumulative`.

## Changing materialization type

When you change a model from `view` to `table` (or vice versa), smelt automatically drops the existing object — regardless of its current type — before creating the new one. You do not need to manually drop the old view before running a model as a table.

```sql
-- Before: view (no frontmatter)
-- After: add frontmatter to change to table

---
materialization: table
---

SELECT ...
```

Run `smelt run --select my_model` and smelt will drop the view and create the table automatically.

## Further reading

- [Incremental Models](incremental-models.md) for time-partitioned incremental processing (requires `table`)
- [cumulative_aggregate spec](../reference/cumulative-aggregate.md) for the normative behaviour, classifier rules, and complete diagnostic list
- [SQL Models](sql-models.md) for YAML frontmatter syntax
