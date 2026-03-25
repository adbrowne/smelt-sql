---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.segment,
    b.user_id,
    c.session_id,
    c.referrer
FROM smelt.ref('sql_l1_122') a
INNER JOIN smelt.ref('sql_l1_74') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l1_349') c ON a.user_id = c.user_id
