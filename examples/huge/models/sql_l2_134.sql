---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.price,
    a.user_id,
    b.session_id
FROM smelt.ref('sql_l1_30') a
INNER JOIN smelt.ref('py_l1_308') b ON a.user_id = b.user_id
