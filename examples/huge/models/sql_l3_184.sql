---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.order_id,
    a.product_id,
    b.os_name
FROM smelt.ref('sql_l2_24') a
LEFT JOIN smelt.ref('py_l2_390') b ON a.user_id = b.user_id
