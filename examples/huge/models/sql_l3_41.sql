---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    referrer,
    RANK() OVER (PARTITION BY event_date ORDER BY created_at) AS win_val
FROM smelt.sql_l2_203
