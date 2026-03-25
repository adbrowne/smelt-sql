---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    category,
    created_at
FROM smelt.ref('orders')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('orders') WHERE created_at >= '2024-01-01'
)
