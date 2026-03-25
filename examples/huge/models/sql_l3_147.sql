---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.os_name,
    a.country,
    b.quantity
FROM smelt.ref('py_l2_407') a
LEFT JOIN smelt.ref('sql_l2_118') b ON a.user_id = b.user_id
