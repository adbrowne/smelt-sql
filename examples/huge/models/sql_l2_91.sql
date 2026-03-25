---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.created_at,
    b.platform,
    c.event_date,
    c.user_id
FROM smelt.ref('py_l1_403') a
INNER JOIN smelt.ref('sql_l1_224') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_49') c ON a.user_id = c.user_id
