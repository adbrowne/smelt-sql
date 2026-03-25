---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    order_id,
    revenue,
    platform
FROM smelt.ref('transactions')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('transactions') WHERE status = 'active'
)
