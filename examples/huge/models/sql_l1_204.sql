---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    cost,
    product_id,
    duration_seconds
FROM smelt.ref('clicks')
WHERE created_at >= '2024-01-01'
