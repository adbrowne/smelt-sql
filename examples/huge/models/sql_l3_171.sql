---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_active,
    discount,
    product_id
FROM smelt.ref('py_l2_476')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_395') WHERE amount > 0
)
