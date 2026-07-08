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
    status,
    ROW_NUMBER() OVER (PARTITION BY updated_at ORDER BY created_at) AS win_val
FROM smelt.refunds
