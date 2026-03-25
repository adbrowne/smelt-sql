---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    price,
    duration_seconds,
    product_id
FROM smelt.ref('sql_l3_149')
WHERE event_type = 'purchase'
