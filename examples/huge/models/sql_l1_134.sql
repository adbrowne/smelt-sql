---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    duration_seconds,
    email_domain,
    event_date,
    channel
FROM smelt.models.categories
WHERE category IS NOT NULL

