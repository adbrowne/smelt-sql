---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.is_verified,
    b.os_name,
    c.tier,
    c.region
FROM smelt.sql_l1_222 a
INNER JOIN smelt.sql_l1_222 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_222 c ON a.user_id = c.user_id

