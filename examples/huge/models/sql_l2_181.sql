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
    is_verified,
    channel,
    ROW_NUMBER() OVER (PARTITION BY is_verified ORDER BY created_at) AS win_val
FROM smelt.sql_l1_36
