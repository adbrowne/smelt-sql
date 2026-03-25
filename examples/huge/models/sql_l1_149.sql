---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    product_id,
    channel,
    platform
FROM smelt.ref('orders')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('orders') WHERE status = 'active'
)
