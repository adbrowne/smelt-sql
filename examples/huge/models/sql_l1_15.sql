---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    product_id,
    plan_type,
    status
FROM smelt.ref('orders')
WHERE created_at >= '2024-01-01'
