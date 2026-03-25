---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_active,
    a.order_id,
    b.product_id
FROM smelt.ref('py_l1_309') a
INNER JOIN smelt.ref('py_l1_309') b ON a.user_id = b.user_id
