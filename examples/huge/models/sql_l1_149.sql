---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    product_id,
    channel,
    platform
FROM smelt.models.orders
WHERE user_id IN (
    SELECT user_id FROM smelt.models.orders WHERE status = 'active'
)

