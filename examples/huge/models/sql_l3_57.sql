---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_verified,
    b.transaction_id,
    c.event_type,
    c.event_date
FROM smelt.ref('py_l2_468') a
INNER JOIN smelt.ref('sql_l2_222') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_418') c ON a.user_id = c.user_id
