---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_active,
    b.platform,
    c.event_time,
    c.referrer
FROM smelt.ref('py_l1_272') a
INNER JOIN smelt.ref('sql_l1_121') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_192') c ON a.user_id = c.user_id
