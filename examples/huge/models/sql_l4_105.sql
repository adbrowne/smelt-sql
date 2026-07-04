---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.is_verified,
    b.channel,
    c.revenue,
    c.device_type
FROM smelt.sql_l3_33 a
INNER JOIN smelt.sql_l3_236 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_17 c ON a.user_id = c.user_id
