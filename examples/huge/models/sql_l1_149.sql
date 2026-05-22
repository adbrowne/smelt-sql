---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    product_id,
    channel,
    platform
FROM smelt.orders
WHERE user_id IN (
    SELECT user_id FROM smelt.orders WHERE status = 'active'
)
