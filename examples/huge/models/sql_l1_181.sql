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
    channel,
    status,
    is_verified
FROM smelt.orders
WHERE user_id IN (
    SELECT user_id FROM smelt.orders WHERE is_active = true
)
