---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.country,
    b.created_at,
    c.cohort_date,
    c.discount
FROM smelt.ref('py_l2_476') a
INNER JOIN smelt.ref('py_l2_294') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_476') c ON a.user_id = c.user_id
