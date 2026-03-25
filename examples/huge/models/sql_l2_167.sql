---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.score,
    b.os_name,
    c.amount,
    c.event_time
FROM smelt.ref('sql_l1_78') a
INNER JOIN smelt.ref('sql_l1_0') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_78') c ON a.user_id = c.user_id
