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

## Materialization compatibility

| Materialization | `timeseries:` allowed? |
|---|---|
| `view` | Yes |
| `table` | Yes |
| `materialized_view` | Yes |
| `ephemeral` | No — `MalformedTimeseries` |
| `test` | No — `MalformedTimeseries` |

## Interaction with `incremental:`

A model that opts into incremental execution (via `incremental: { enabled: true }`) must also declare `timeseries:`. The two blocks are independent:

- `timeseries:` declares the time dimension (event time, partition column, granularity).
- `incremental:` opts the model into incremental execution and carries strategy keys (`unique_key`, `safety_overrides`).

Declaring `incremental:` without `timeseries:` is a validation error. See the [incremental models guide](../guide/incremental-models.md).

## Diagnostic codes

| Code | Severity | Trigger |
|---|---|---|
| `MalformedTimeseries` | Error | The `timeseries:` block parses but violates a structural rule — unknown key, `granularity` not in the enum, `partition_column` absent from the model's SQL body, `week_start` set without `granularity: week`, or `timeseries:` on an `ephemeral` / `test` model. |
| `TimeseriesRequiredForIncremental` | Error | A model declares `incremental:` but has no `timeseries:` block. |
