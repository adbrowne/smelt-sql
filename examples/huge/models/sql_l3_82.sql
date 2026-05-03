---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    revenue,
    event_time,
    LAG(amount, 1) OVER (PARTITION BY revenue ORDER BY created_at) AS win_val
FROM smelt.sql_l2_43

