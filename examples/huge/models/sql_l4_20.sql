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
    device_type,
    profit,
    RANK() OVER (PARTITION BY device_type ORDER BY created_at) AS win_val
FROM smelt.sql_l3_155
