---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_time,
    score,
    region,
    segment
FROM smelt.sql_l1_42
WHERE is_active = true
