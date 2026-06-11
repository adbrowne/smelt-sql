# SQL Models

smelt models are SQL files in the `models/` directory with optional YAML frontmatter for configuration.

## Basic model

```sql
SELECT
  user_id,
  COUNT(*) as event_count
FROM smelt.events
GROUP BY 1
```

## YAML frontmatter

Add configuration inline using YAML frontmatter:

```sql
---
name: user_activity
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
tags: [users, daily]
owner: analytics-team
description: Daily user activity summary
---

SELECT
  DATE(event_time) as event_date,
  user_id,
  COUNT(*) as event_count
FROM smelt.events
GROUP BY 1, 2
```

## References

Use `smelt.<name>` to reference other models and seeds. Addressing is **flat** — seeds and models share the same namespace with no intermediate segment:

```sql
SELECT * FROM smelt.upstream_model
SELECT * FROM smelt.my_seed  -- seeds are first-class ref targets
```

The parser supports named parameters using `=>` syntax:

```sql
SELECT * FROM smelt.events(filter => date > '2024-01-01')
```

!!! note
    Named parameter support in `smelt.<name>` is parsed but not yet used at runtime. The primary use case is `smelt.model_name`.

For more on defining external sources, see [Sources](sources.md).

## Sources

Use `smelt.sources.<name>` for external tables declared as per-entity `.yml` files under `paths:`:

```sql
SELECT * FROM smelt.sources.raw.users
```

## Supported SQL features

smelt's type inference and LSP diagnostics understand the following SQL patterns:

### Common Table Expressions

```sql
WITH
  filtered AS (
    SELECT * FROM smelt.events WHERE event_type = 'purchase'
  ),
  summary AS (
    SELECT user_id, COUNT(*) AS purchase_count FROM filtered GROUP BY 1
  )
SELECT * FROM summary
```

### CASE expressions

```sql
SELECT
  user_id,
  CASE
    WHEN total_spent > 1000 THEN 'high_value'
    WHEN total_spent > 100  THEN 'medium_value'
    ELSE 'low_value'
  END AS value_segment
FROM smelt.user_totals
```

### EXTRACT

```sql
SELECT
  EXTRACT(YEAR FROM event_timestamp) AS event_year,
  EXTRACT(EPOCH FROM event_timestamp) AS unix_ts
FROM smelt.events
```

### Subqueries

```sql
SELECT *
FROM (
  SELECT user_id, SUM(amount) AS total
  FROM smelt.transactions
  GROUP BY 1
) AS sub
WHERE sub.total > 100
```

## Aggregate type gotchas

**`COUNT(*)` returns `BIGINT`, not `INTEGER`.** If a downstream model or acceptance check expects `INTEGER`, cast explicitly:

```sql
CAST(COUNT(*) AS INTEGER) AS order_count
```

**`COALESCE(SUM(col), 0.0)` returns `DECIMAL(38,2)`, not `DOUBLE`.** DuckDB promotes to the wider decimal type when the fallback literal is `0.0`. If you need `DOUBLE`:

```sql
CAST(COALESCE(SUM(col), 0.0) AS DOUBLE) AS revenue
```

## Decimal division

`Decimal / T` (dividing a `DECIMAL` column or literal by any numeric type) is not in the portable surface and produces a `TypeMismatch` diagnostic. The portable remedy is to cast both operands to `DOUBLE` before dividing:

```sql
-- Not portable — emits TypeMismatch
SELECT price / 100.0 AS unit_price   -- price is DECIMAL

-- Portable — use DOUBLE arithmetic
SELECT CAST(price AS DOUBLE) / 100.0 AS unit_price
-- or cast the denominator too for full clarity
SELECT CAST(price AS DOUBLE) / CAST(100.0 AS DOUBLE) AS unit_price
```

Integer-on-integer division (`INTEGER / INTEGER → INTEGER`) and floating-point division (`DOUBLE / DOUBLE → DOUBLE`) are unaffected.

## Configuration precedence

**SQL frontmatter > smelt.yml > defaults**

Frontmatter in SQL files overrides project-level `smelt.yml` settings.

## Supported metadata fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Model name (optional, inferred from filename) |
| `materialization` | `table` \| `view` \| `ephemeral` \| `materialized_view` | How to materialize. See [Materializations](materializations.md) for details on each type. |
| `incremental.enabled` | boolean | Enable incremental updates |
| `incremental.event_time_column` | string | Column for time-based filtering |
| `incremental.partition_column` | string | Column for partition deletion |
| `incremental.granularity` | `hour` \| `day` \| `week` \| `month` \| `quarter` \| `year` | Time granularity for partitioning |
| `incremental.unique_key` | string \| string[] | Columns for row-level merge (optional) |
| `schema_evolution` | object | Schema-change strategy for incremental models. Controls how smelt handles output schema changes (e.g., `alter_and_backfill` or `full_refresh`). See [Schema Evolution](schema-evolution.md). |
| `columns` | object/map | Per-column metadata (defaults and backfill expressions) used during schema evolution. See [Schema Evolution](schema-evolution.md). |
| `format` | `delta` \| `parquet` | Per-model table format override for Spark targets. Affects schema evolution capabilities. See [Schema Evolution — Table format configuration](schema-evolution.md#table-format-configuration). |
| `tags` | string[] | Organization tags |
| `owner` | string | Responsible team or person |
| `description` | string | Model documentation |

See [Incremental Models](incremental-models.md) for a complete guide.
