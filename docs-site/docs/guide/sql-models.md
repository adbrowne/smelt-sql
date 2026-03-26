# SQL Models

smelt models are SQL files in the `models/` directory with optional YAML frontmatter for configuration.

## Basic model

```sql
SELECT
  user_id,
  COUNT(*) as event_count
FROM smelt.ref('events')
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
FROM smelt.ref('events')
GROUP BY 1, 2
```

## References

Use `smelt.ref()` to reference other models:

```sql
SELECT * FROM smelt.ref('upstream_model')
```

The parser supports named parameters using `=>` syntax:

```sql
SELECT * FROM smelt.ref('events', filter => date > '2024-01-01')
```

!!! note
    Named parameter support in `smelt.ref()` is parsed but not yet used at runtime. The primary use case is `smelt.ref('model_name')`.

For more on defining external sources, see [Sources](sources.md).

## Sources

Use `smelt.source()` for external tables defined in `sources.yml`:

```sql
SELECT * FROM smelt.source('raw.users')
```

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

See [Incremental Models](incremental-models.md) for a complete guide.

| `tags` | string[] | Organization tags |
| `owner` | string | Responsible team or person |
| `description` | string | Model documentation |
