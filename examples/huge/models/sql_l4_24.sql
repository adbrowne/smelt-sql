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
    status,
    referrer,
    RANK() OVER (PARTITION BY status ORDER BY created_at) AS win_val
FROM smelt.sql_l3_247
