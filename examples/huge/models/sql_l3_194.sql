---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.created_at,
    b.category,
    c.platform,
    c.cohort_date
FROM smelt.ref('py_l2_455') a
INNER JOIN smelt.ref('sql_l2_116') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_455') c ON a.user_id = c.user_id
