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
    category,
    browser,
    RANK() OVER (PARTITION BY category ORDER BY created_at) AS win_val
FROM smelt.sql_l2_18
