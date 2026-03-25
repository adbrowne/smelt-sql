---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    order_id,
    country,
    is_active
FROM smelt.ref('reviews')
WHERE is_active = true
