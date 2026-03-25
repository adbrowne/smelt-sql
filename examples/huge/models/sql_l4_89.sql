---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.product_id,
    a.created_at,
    b.cohort_date
FROM smelt.ref('py_l3_250') a
INNER JOIN smelt.ref('sql_l3_244') b ON a.user_id = b.user_id
