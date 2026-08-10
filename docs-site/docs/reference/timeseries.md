# Timeseries Reference

`timeseries:` declares the time dimension on a model's or source's output. Downstream consumers — the incremental planner, the CLI, the LSP — use this declaration to understand partition boundaries, push down time filters, and derive lookback windows.

## Frontmatter block (SQL models)

```sql
---
materialization: table
timeseries:
  event_time_column: order_ts        # source-of-truth time column
  partition_column: order_date       # column the engine prunes on
  granularity: day                   # hour | day | week | month | quarter | year
---

SELECT DATE_TRUNC('day', order_ts) AS order_date, order_ts, customer_id, amount
FROM smelt.orders_raw
```

## Keys

| Key | Required | Default | Type / Values |
|---|---|---|---|
| `event_time_column` | yes | — | Identifier — name of the time column on the model's output (timestamp or date) |
| `partition_column` | yes | — | Identifier — name of the column the engine prunes on (date or integer) |
| `granularity` | yes | — | One of: `hour`, `day`, `week`, `month`, `quarter`, `year` |
| `week_start` | no | `monday` | When `granularity: week`. One of: `monday`, `sunday` |

`event_time_column` and `partition_column` may be the same column. They differ when the source-of-truth time is a timestamp and the partition is a derived date (e.g., `DATE_TRUNC('day', order_ts)`).

### Granularity options

- **`hour`** — one partition per hour.
- **`day`** — one partition per calendar day (most common).
- **`week`** — one partition per week. Supports a configurable week-start day:
  ```yaml
  timeseries:
    granularity: week
    week_start: monday   # or: sunday
  ```
- **`month`** — one partition per calendar month.
- **`quarter`** — one partition per calendar quarter.
- **`year`** — one partition per calendar year.

## Source YAML block

The same keys apply when declaring a time dimension on an external source:

```yaml
description: Raw orders feed.
columns:
  - { name: order_id,   type: INTEGER,            nullable: false }
  - { name: order_ts,   type: TIMESTAMP,           nullable: false }
  - { name: order_date, type: DATE,                nullable: false }
  - { name: customer_id, type: INTEGER,            nullable: false }
  - { name: amount,     type: DECIMAL(18,2),       nullable: false }
timeseries:
  event_time_column: order_ts
  partition_column: order_date
  granularity: day
```

A source that declares `timeseries:` must list the named `event_time_column` and `partition_column` in its `columns:` block.

## `smelt.yml` overrides

```yaml
models:
  daily_revenue:
    timeseries:
      event_time_column: order_ts
      partition_column: order_date
      granularity: day
```

Frontmatter wins over `smelt.yml` when both set the same field. The two sources are merged key-by-key: declaring `granularity: day` in frontmatter and `event_time_column: order_ts` in `smelt.yml` yields a single combined block.

## Materialization / refresh compatibility

| Storage / refresh | `timeseries:` allowed? |
|---|---|
| `view` | Yes |
| `table`, `refresh: full` | Yes |
| `table`, `refresh: incremental`, `grain: partition` | **Required** |
| `table`, `refresh: incremental`, `grain: key` | **Optional** — the [composed shape](#interaction-with-grain-key): admitted iff [key temporal locality](../guide/incremental-models.md#the-composed-shape-key-time) is established, refused otherwise (`KeyedForbidsTimeseries`, naming the missing route) |
| `table`, `refresh: incremental`, derived `key_per_partition` grain | **Required** — the partition axis is half the grain |
| `table`, `refresh: materialized_view` | No — `MaterializedViewForbidsTimeseries` |
| `ephemeral` | No — `MalformedTimeseries` |
| `test` | No — `MalformedTimeseries` |

## Interaction with `refresh: incremental`

A `refresh: incremental` model must also declare `timeseries:` whenever its grain (declared or derived) is `partition` or `key_per_partition`. The two blocks are independent:

- `timeseries:` declares the time dimension (event time, partition column, granularity).
- `refresh: incremental` + `grain:` opts the model into the derived maintenance plan; the top-level `safety_overrides:` key carries per-model safety-check escape hatches layered on top.

Declaring `refresh: incremental` + `grain: partition` without `timeseries:` is a validation error. See the [incremental models guide](../guide/incremental-models.md).

## Interaction with `grain: key`

The key axis (identity, via `unique_key:`) and the time axis (clock, via `timeseries:`) are independent — a model can declare either, both, or neither. Declaring both is the **composed shape**: a keyed output (one merged row per key) that is also time-partitioned. It is not the default for a `grain: key` model — by default a keyed output has no partition column at all — and it is not automatic: admitting a `timeseries:` block on a keyed output additionally requires **key temporal locality**, a proof or a checked declaration that every duplicate delivery of one key stays within a bounded window of itself on the event axis. Three routes establish it:

1. **Key-embedded** — `partition_column` is itself a `unique_key` column.
2. **Key-determined** — `partition_column` is proven a per-key constant by a declared `functional_dependencies:` entry naming it.
3. **Recurrence-bounded** — a `key_recurrence` window declared on the driving source's `mutation_profile:` (see [Declaring how a source mutates](../guide/sources.md#declaring-how-a-source-mutates) and the [source YAML reference](../reference/sources-yml.md#mutation-profile)), checked transactionally at merge time rather than trusted.

A `grain: key` model whose `timeseries:` block satisfies none of the three routes is refused (`KeyedForbidsTimeseries`, naming all three routes and the nearest missing fact) rather than silently falling back to a bare keyed output. See [the composed shape](../guide/incremental-models.md#the-composed-shape-key-time) for the full walkthrough, and the [deduplication tutorial](../examples/web-analytics/deduplication.md) for a worked example using the recurrence-bounded route.

## Diagnostic codes

| Code | Severity | Trigger |
|---|---|---|
| `MalformedTimeseries` | Error | The `timeseries:` block parses but violates a structural rule — unknown key, `granularity` not in the enum, `partition_column` absent from the model's SQL body, `week_start` set without `granularity: week`, or `timeseries:` on an `ephemeral` / `test` model. |
| `TimeseriesRequiredForPartitionGrain` | Error | A model declares `refresh: incremental` + `grain: partition` but has no `timeseries:` block. |
