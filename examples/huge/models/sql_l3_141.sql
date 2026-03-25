---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.cohort_date,
    b.session_id,
    c.category,
    c.region
FROM smelt.ref('py_l2_424') a
INNER JOIN smelt.ref('py_l2_297') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_490') c ON a.user_id = c.user_id
