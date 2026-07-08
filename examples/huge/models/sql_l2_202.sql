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
    created_at,
    os_name,
    ip_address
FROM smelt.sql_l1_16
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_82 WHERE created_at >= '2024-01-01'
)
