---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.discount,
    b.email_domain,
    c.price,
    c.device_type
FROM smelt.models.sql_l3_228 a
INNER JOIN smelt.models.sql_l3_243 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l3_228 c ON a.user_id = c.user_id

