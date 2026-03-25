---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.session_id,
    b.category,
    c.platform,
    c.user_id
FROM smelt.ref('py_l2_352') a
INNER JOIN smelt.ref('sql_l2_225') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_341') c ON a.user_id = c.user_id
