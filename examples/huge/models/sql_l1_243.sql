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
    device_type,
    price
FROM smelt.models.refunds
WHERE user_id IN (
    SELECT user_id FROM smelt.models.refunds WHERE status = 'active'
)

