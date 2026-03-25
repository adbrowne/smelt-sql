---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.user_id,
    a.quantity,
    b.product_id
FROM smelt.ref('py_l2_291') a
INNER JOIN smelt.ref('sql_l2_200') b ON a.user_id = b.user_id
