---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.plan_type,
    b.segment,
    c.device_type,
    c.country
FROM smelt.ref('sql_l2_188') a
INNER JOIN smelt.ref('py_l2_291') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_312') c ON a.user_id = c.user_id
