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
    os_name
FROM smelt.sql_l2_54
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_110 WHERE is_active = true
)
