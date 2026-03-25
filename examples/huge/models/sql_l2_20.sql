---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.cohort_date,
    a.updated_at,
    b.is_active
FROM smelt.ref('py_l1_282') a
LEFT JOIN smelt.ref('py_l1_298') b ON a.user_id = b.user_id
