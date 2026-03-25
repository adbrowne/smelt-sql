---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.category,
    a.device_type,
    b.user_id
FROM smelt.ref('py_l3_278') a
INNER JOIN smelt.ref('sql_l3_163') b ON a.user_id = b.user_id
