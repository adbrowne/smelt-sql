---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.product_id,
    b.platform,
    c.plan_type,
    c.cohort_date
FROM smelt.ref('py_l1_348') a
INNER JOIN smelt.ref('sql_l1_16') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_37') c ON a.user_id = c.user_id
