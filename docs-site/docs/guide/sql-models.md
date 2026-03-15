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

Named parameters are supported:

```sql
SELECT * FROM smelt.ref('events', filter => date > '2024-01-01', limit => 1000)
```

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
| `materialization` | `table` \| `view` | How to materialize |
| `incremental.enabled` | boolean | Enable incremental updates |
| `incremental.event_time_column` | string | Column for time-based filtering |
| `incremental.partition_column` | string | Column for partition deletion |
| `tags` | string[] | Organization tags |
| `owner` | string | Responsible team or person |
| `description` | string | Model documentation |
