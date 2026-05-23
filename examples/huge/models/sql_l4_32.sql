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
    a.plan_type,
    a.device_type,
    b.rating
FROM smelt.sql_l3_57 a
LEFT JOIN smelt.sql_l3_238 b ON a.user_id = b.user_id
