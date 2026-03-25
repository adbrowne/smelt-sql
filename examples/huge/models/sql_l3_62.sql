---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.score,
    b.event_type,
    c.status,
    c.country
FROM smelt.ref('sql_l2_136') a
INNER JOIN smelt.ref('py_l2_326') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_136') c ON a.user_id = c.user_id
