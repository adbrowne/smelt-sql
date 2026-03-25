---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    product_id,
    is_verified,
    price,
    rating
FROM smelt.ref('py_l1_377')
WHERE status = 'active'
