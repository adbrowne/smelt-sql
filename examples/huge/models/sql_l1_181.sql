---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    status,
    is_verified
FROM smelt.ref('orders')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('orders') WHERE is_active = true
)
