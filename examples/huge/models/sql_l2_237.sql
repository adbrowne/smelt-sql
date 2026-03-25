---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.session_id,
    a.os_name,
    b.tier
FROM smelt.ref('sql_l1_18') a
LEFT JOIN smelt.ref('py_l1_261') b ON a.user_id = b.user_id
