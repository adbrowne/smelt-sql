---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.device_type,
    b.duration_seconds,
    c.tier,
    c.referrer
FROM smelt.models.sql_l1_181 a
INNER JOIN smelt.models.sql_l1_134 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l1_181 c ON a.user_id = c.user_id

