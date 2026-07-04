---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_verified,
    score,
    LAG(amount, 1) OVER (PARTITION BY is_verified ORDER BY created_at) AS win_val
FROM smelt.sql_l2_87
