---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.channel,
    b.price,
    c.device_type,
    c.event_date
FROM smelt.ref('py_l2_299') a
INNER JOIN smelt.ref('py_l2_276') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_218') c ON a.user_id = c.user_id
