---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.score,
    b.os_name,
    c.amount,
    c.event_time
FROM smelt.ref('sql_l1_209') a
INNER JOIN smelt.ref('sql_l1_209') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_209') c ON a.user_id = c.user_id
