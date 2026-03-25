---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.updated_at,
    b.transaction_id,
    c.cohort_date,
    c.cost
FROM smelt.ref('py_l3_271') a
INNER JOIN smelt.ref('py_l3_271') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l3_271') c ON a.user_id = c.user_id
