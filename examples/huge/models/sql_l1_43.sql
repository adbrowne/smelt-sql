---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    referrer,
    email_domain,
    duration_seconds,
    category
FROM smelt.categories
WHERE quantity > 0
