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
    is_active,
    referrer,
    country,
    price
FROM smelt.sql_l2_102
WHERE platform = 'web'
