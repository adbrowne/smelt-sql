---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.created_at,
    b.os_name,
    c.user_id,
    c.score
FROM smelt.ref('py_l2_486') a
INNER JOIN smelt.ref('py_l2_450') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_486') c ON a.user_id = c.user_id
