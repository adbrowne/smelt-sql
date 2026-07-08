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
    product_id,
    device_type,
    price
FROM smelt.refunds
WHERE user_id IN (
    SELECT user_id FROM smelt.refunds WHERE status = 'active'
)
