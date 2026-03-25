---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    transaction_id,
    rating,
    product_id
FROM smelt.ref('sql_l1_150')
WHERE status = 'active'
