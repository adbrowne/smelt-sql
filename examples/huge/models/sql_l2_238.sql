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
    event_time,
    platform,
    RANK() OVER (PARTITION BY event_time ORDER BY created_at) AS win_val
FROM smelt.sql_l1_216
