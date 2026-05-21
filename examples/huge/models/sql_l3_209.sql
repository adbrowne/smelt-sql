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
    ip_address,
    event_date,
    LAG(amount, 1) OVER (PARTITION BY ip_address ORDER BY created_at) AS win_val
FROM smelt.sql_l2_120

