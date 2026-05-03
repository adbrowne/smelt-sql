---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.ip_address,
    b.is_active,
    c.cost,
    c.device_type
FROM smelt.sql_l3_76 a
INNER JOIN smelt.sql_l3_42 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_99 c ON a.user_id = c.user_id

