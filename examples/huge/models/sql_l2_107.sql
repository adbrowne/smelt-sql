---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.order_id,
    b.browser,
    c.plan_type,
    c.cohort_date
FROM smelt.ref('sql_l1_99') a
INNER JOIN smelt.ref('py_l1_306') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_99') c ON a.user_id = c.user_id
