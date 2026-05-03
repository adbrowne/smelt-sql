---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.session_id,
    b.category,
    c.platform,
    c.user_id
FROM smelt.sql_l2_64 a
INNER JOIN smelt.sql_l2_182 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_64 c ON a.user_id = c.user_id

