---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.product_id,
    a.amount,
    b.quantity
FROM smelt.ref('py_l2_313') a
LEFT JOIN smelt.ref('py_l2_286') b ON a.user_id = b.user_id
