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
    tier,
    updated_at,
    RANK() OVER (PARTITION BY tier ORDER BY created_at) AS win_val
FROM smelt.sql_l3_190
