---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.plan_type,
    b.segment,
    c.device_type,
    c.country
FROM smelt.sql_l2_237 a
INNER JOIN smelt.sql_l2_80 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_19 c ON a.user_id = c.user_id
