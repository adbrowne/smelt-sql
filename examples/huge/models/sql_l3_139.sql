---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.created_at,
    a.referrer,
    b.user_id
FROM smelt.ref('sql_l2_207') a
LEFT JOIN smelt.ref('py_l2_415') b ON a.user_id = b.user_id
