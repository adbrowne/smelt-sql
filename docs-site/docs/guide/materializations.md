# Materializations

A model is described by three independent axes: **kind** (model, test, function, …), **materialization** (how storage works: `view` / `table` / `ephemeral`), and **refresh** (how a stored `table` is kept current across runs: `full` / `incremental` / `materialized_view`). This page covers materialization first, then the refresh axis.

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
- `refresh: incremental` models (every `grain:` requires `table` materialization)

### ephemeral

Not materialized at all. The model's SQL is inlined as a CTE (Common Table Expression) into every downstream model that references it.

Best for:

- Intermediate transformation steps that don't need to be queried directly
- Reducing the number of objects in your database
- Simple column renames or type casts

!!! warning
    Ephemeral models cannot declare `refresh: incremental` / `grain:`, or target overrides. smelt will raise an error if you try to combine these.

Key-grain running state (one row per `GROUP BY` key, folded across partitions) is not a fourth materialization type — it is `materialization: table` + `refresh: incremental` + `grain: key`. See [Refresh axis](#refresh-axis) below.

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

The `refresh:` frontmatter key controls how a stored model's output is recomputed on each run. It applies only to `materialization: table`; setting it on other materialization types has no effect for `view` (a warning is emitted) and is a hard error for `ephemeral`. There is no `materialization: materialized_view` storage type — a backend-managed materialized view is selected on the refresh axis instead, via `refresh: materialized_view` over an implied `table`.

| Value | Who keeps it current | Contract |
|---|---|---|
| `full` (default) | smelt, by recomputing everything each run | trivial (recompute) |
| `incremental` | smelt, by running the derived maintenance plan each run | processed-input equivalence, discharged per cell — see [Incremental Models](incremental-models.md) |
| `materialized_view` | the engine, continuously, via native incremental-view maintenance | end-state; engine-owned — requires a backend with native IVM, see below |

When `refresh:` is omitted, `full` is assumed — the model always rebuilds completely. `refresh: incremental` is admitted on its two shape-defining facts (`timeseries:` and/or `unique_key:`), which determine a derived `grain` label — what a stored row *is* and how it is addressed. `grain: partition` and `grain: key` may also be written as a check-only assertion; `key_per_partition` has no writable spelling:

| `grain` (derived label) | A stored row is… | Identity | `timeseries:` |
|---|---|---|---|
| `partition` | one row of a complete, partition-addressed table | `unique_key` optional (within-partition dedup aid only) | **required** |
| `key` | the end-state per key | `unique_key` **required**, composite-valued | **forbidden** — the output has no partition column; partition shape is read from the source |
| `key_per_partition` (derived-only) | the trajectory: one row per `(key, partition)` | `unique_key` **required** | **required** — the partition axis is half the grain |

There is no per-model `strategy:` knob that says "delete+insert", "merge", or "fold" — how each part of the output is maintained under each kind of change is *derived* per `(column-group × trigger)` cell (the maintenance plan) and reported by `smelt explain`. One model is routinely append-driven, merge-driven, and recompute-driven at different cells.

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

### refresh: incremental + grain: partition

Processes only new or changed data instead of rebuilding the entire table, keeping one row per partition. See [Incremental Models](incremental-models.md) for the full DELETE+INSERT mechanics, safety analysis, and configuration.

```sql
---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
---
SELECT event_date, device_id, user_id, COUNT(*) AS event_count
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY event_date, device_id, user_id
```

### refresh: incremental + grain: key

Stateful merge into one row per `GROUP BY` key. Each run only aggregates the new partition's events and merges them into the running keyed state via `merge_into`. The unique key, the per-column aggregator, and the cross-window combiner are all derived from the SELECT.

```sql
---
materialization: table
refresh: incremental
grain: key
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

Each non-key projection must be a direct call to one of the **allowlisted aggregators**, which are paired with a fixed cross-window combiner:

| Per-partition aggregator | Cross-window combiner |
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

Anything outside the allowlist — `STRING_AGG`, `LIST_AGG`, `FIRST`, `LAST`, `AVG`, composite expressions like `SUM(x) + 1` — is rejected at planning time. Each allowed aggregator is commutative and associative, which is the property that lets the rule merge windows in any order and still produce the same final state.

Best for:

- Running counts and identity edge sets (e.g. `(device, user)` co-occurrence)
- Per-key rollups where each run should be cheap — proportional to the new window's source rows, not the full source history
- Tables consumed downstream as a lookup (no `partition_column` on the output — the output has no partition column at all)

!!! warning "Forbidden combinations"
    The retired `batched:` sub-block is a hard error for every model, regardless of grain — see [Incremental models](incremental-models.md). Top-level `safety_overrides:` is admitted only on a partition-shaped output (no declared identity); declaring it on a `grain: key` model is refused. `refresh: incremental` on an `ephemeral` model is also a hard error.

`grain: key` and `timeseries:` are **not** a forbidden combination — the key axis (identity) and the time axis (clock) are independent, and declaring both is a first-class shape of its own: the [composed shape](incremental-models.md#the-composed-shape-key-time), a keyed output that is also time-partitioned. It requires one more fact beyond a bare `grain: key` declaration: proof, or a checked declaration, that key temporal locality holds (every duplicate delivery of one key stays within a bounded window of itself on the event axis). A `timeseries:` block on a `grain: key` model that satisfies none of the three locality routes is refused (`KeyedForbidsTimeseries`, naming the missing route) — the composed shape is opt-in on the declared facts, not automatic.

!!! note "Reprocessing"
    Reprocessing an already-merged window is refused when detected. If a past window's data changes, run with `--full-refresh` to truncate and rebuild from scratch.

For the deeper rationale, see the [key-grain patterns reference](../reference/cumulative-aggregate.md).

### refresh: materialized_view

Delegates freshness to the backend's own native incremental-view maintenance instead of a smelt-driven refresh loop. The output is a keyed lookup, like `grain: key`; it must not declare a `timeseries:` or `grain:` key.

```sql
---
materialization: table
refresh: materialized_view
---
SELECT device_id, user_id, COUNT(*) AS event_count
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
```

smelt never silently substitutes another refresh mode for this one: on a backend without native incremental-view maintenance (every backend today — DuckDB and both Spark profiles), `refresh: materialized_view` is a **hard error** rather than a silent fallback to `grain: key` or a full-refresh table. Use `refresh: incremental` + `grain: key` for smelt-driven maintenance on those backends.

## Decision guide

| Scenario | Recommended |
|---|---|
| Staging layer / light transforms | `view` |
| Aggregations queried by BI tools | `table` |
| Intermediate step used by one downstream | `ephemeral` |
| Model with many downstream dependents | `table` |
| Incremental processing (per-partition output) | `table` + `refresh: incremental` + `grain: partition` |
| Keyed state (one row per key across history) | `table` + `refresh: incremental` + `grain: key` |
| Database-managed refresh | `table` + `refresh: materialized_view` |
| Development / iteration | `view` |

## grain: partition vs grain: key

Both shapes are time-aware, but they uphold different contracts:

| Property | `grain: partition` | `grain: key` |
|---|---|---|
| Frontmatter | `materialization: table` + `refresh: incremental` + `grain: partition` + `timeseries:` | `materialization: table` + `refresh: incremental` + `grain: key` |
| Output shape | One row per `(partition_column, …)` — partitioned | One row per `GROUP BY` key — collapsed (by default) |
| Declares `timeseries:`? | Yes (the model's output is a timeseries) | By default, no (reads partition shape from the source) — **may** also declare `timeseries:` to become the [composed shape](incremental-models.md#the-composed-shape-key-time) once key temporal locality is established; refused (`KeyedForbidsTimeseries`) otherwise |
| Equivalence contract | Per-partition equivalence with full refresh | End-state equivalence under any admitted window ordering |
| Re-running a past window | Idempotent (DELETE+INSERT) | Refused when detected; use `--full-refresh` |
| Backend primitive | `DELETE` + `INSERT` per partition | `MERGE INTO` with per-column combiners |

If the question is "what's the day's contribution?", use `grain: partition`. If the question is "what's the running total per key?", use `grain: key`.

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

- [Incremental Models](incremental-models.md) for time-partitioned incremental processing (`grain: partition`, requires `table`)
- [Key-grain patterns reference](../reference/cumulative-aggregate.md) for the normative behaviour, classifier rules, and complete diagnostic list of `grain: key`
- [SQL Models](sql-models.md) for YAML frontmatter syntax
