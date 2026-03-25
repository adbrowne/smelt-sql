---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.ip_address,
    b.is_active,
    c.cost,
    c.device_type
FROM smelt.ref('sql_l3_44') a
INNER JOIN smelt.ref('py_l3_358') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_230') c ON a.user_id = c.user_id
