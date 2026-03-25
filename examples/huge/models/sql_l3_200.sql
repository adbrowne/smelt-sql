---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.status,
    a.rating,
    b.updated_at
FROM smelt.ref('py_l2_330') a
LEFT JOIN smelt.ref('sql_l2_226') b ON a.user_id = b.user_id
