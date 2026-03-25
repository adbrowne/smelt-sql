---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.plan_type,
    a.device_type,
    b.rating
FROM smelt.ref('py_l3_263') a
LEFT JOIN smelt.ref('py_l3_263') b ON a.user_id = b.user_id
