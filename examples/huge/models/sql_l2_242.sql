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
    rating,
    ip_address,
    LAG(amount, 1) OVER (PARTITION BY rating ORDER BY created_at) AS win_val
FROM smelt.sql_l1_8
