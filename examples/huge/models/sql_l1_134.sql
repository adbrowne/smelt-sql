---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    duration_seconds,
    email_domain,
    event_date,
    channel
FROM smelt.categories
WHERE category IS NOT NULL

