---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    duration_seconds,
    discount,
    segment,
    cost
FROM smelt.sql_l2_49
WHERE platform = 'web'
