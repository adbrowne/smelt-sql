---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_verified,
    a.created_at,
    b.status
FROM smelt.ref('py_l1_311') a
INNER JOIN smelt.ref('py_l1_311') b ON a.user_id = b.user_id
