---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_verified,
    a.status,
    b.quantity
FROM smelt.ref('py_l3_285') a
LEFT JOIN smelt.ref('py_l3_285') b ON a.user_id = b.user_id
