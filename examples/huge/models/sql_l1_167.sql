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
    updated_at,
    device_type,
    ROW_NUMBER() OVER (PARTITION BY updated_at ORDER BY created_at) AS win_val
FROM smelt.logs
