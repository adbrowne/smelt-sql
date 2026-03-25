---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    discount,
    product_id,
    category
FROM smelt.ref('sql_l1_248')
WHERE is_active = true
