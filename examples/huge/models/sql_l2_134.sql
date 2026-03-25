---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.price,
    a.user_id,
    b.session_id
FROM smelt.ref('sql_l1_71') a
INNER JOIN smelt.ref('sql_l1_71') b ON a.user_id = b.user_id
