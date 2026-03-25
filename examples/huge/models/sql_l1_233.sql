---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_type,
    platform,
    transaction_id,
    quantity
FROM smelt.ref('products')
WHERE score >= 50
