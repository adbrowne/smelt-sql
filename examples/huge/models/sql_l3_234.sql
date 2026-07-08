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
    cost,
    user_id,
    RANK() OVER (PARTITION BY cost ORDER BY created_at) AS win_val
FROM smelt.sql_l2_160
