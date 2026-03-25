---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    product_id,
    device_type,
    price
FROM smelt.ref('refunds')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('refunds') WHERE status = 'active'
)
