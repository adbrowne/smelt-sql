---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.channel,
    b.price,
    c.device_type,
    c.event_date
FROM smelt.models.sql_l2_97 a
INNER JOIN smelt.models.sql_l2_180 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l2_97 c ON a.user_id = c.user_id

