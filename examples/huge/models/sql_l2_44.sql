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
    session_id,
    country,
    LAG(amount, 1) OVER (PARTITION BY session_id ORDER BY created_at) AS win_val
FROM smelt.sql_l1_66
