---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    device_type,
    score,
    LAG(amount, 1) OVER (PARTITION BY device_type ORDER BY created_at) AS win_val
FROM smelt.sql_l1_94

