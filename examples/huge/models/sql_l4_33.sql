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
    a.is_verified,
    a.status,
    b.quantity
FROM smelt.sql_l3_240 a
LEFT JOIN smelt.sql_l3_240 b ON a.user_id = b.user_id

