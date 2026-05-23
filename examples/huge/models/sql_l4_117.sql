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
    duration_seconds,
    price,
    RANK() OVER (PARTITION BY duration_seconds ORDER BY created_at) AS win_val
FROM smelt.sql_l3_67
