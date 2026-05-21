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
    segment,
    duration_seconds,
    LAG(amount, 1) OVER (PARTITION BY segment ORDER BY created_at) AS win_val
FROM smelt.sql_l3_7

