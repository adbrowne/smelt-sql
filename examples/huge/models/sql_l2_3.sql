---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.transaction_id,
    a.event_type,
    b.os_name
FROM smelt.ref('py_l1_283') a
LEFT JOIN smelt.ref('sql_l1_56') b ON a.user_id = b.user_id
