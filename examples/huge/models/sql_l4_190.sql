---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_active,
    browser,
    RANK() OVER (PARTITION BY is_active ORDER BY created_at) AS win_val
FROM smelt.sql_l3_247
