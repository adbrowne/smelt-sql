---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.transaction_id,
    b.is_verified,
    c.rating,
    c.ip_address
FROM smelt.ref('py_l1_349') a
INNER JOIN smelt.ref('py_l1_438') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l1_349') c ON a.user_id = c.user_id
