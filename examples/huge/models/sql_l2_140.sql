---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_time,
    a.plan_type,
    b.browser
FROM smelt.ref('py_l1_266') a
INNER JOIN smelt.ref('sql_l1_142') b ON a.user_id = b.user_id
