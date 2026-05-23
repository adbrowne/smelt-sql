---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.price,
    b.device_type,
    c.amount,
    c.created_at
FROM smelt.sql_l1_3 a
INNER JOIN smelt.sql_l1_180 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_144 c ON a.user_id = c.user_id
