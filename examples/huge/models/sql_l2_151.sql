---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_type,
    a.is_verified,
    b.ip_address
FROM smelt.ref('py_l1_251') a
LEFT JOIN smelt.ref('sql_l1_189') b ON a.user_id = b.user_id
