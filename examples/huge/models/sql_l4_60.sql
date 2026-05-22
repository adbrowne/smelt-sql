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
    country,
    referrer,
    created_at,
    event_time
FROM smelt.sql_l3_4
WHERE quantity > 0
