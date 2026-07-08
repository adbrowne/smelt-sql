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
    profit,
    os_name,
    device_type
FROM smelt.users
WHERE user_id IN (
    SELECT user_id FROM smelt.users WHERE created_at >= '2024-01-01'
)
