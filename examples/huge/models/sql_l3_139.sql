---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.created_at,
    a.referrer,
    b.user_id
FROM smelt.sql_l2_57 a
LEFT JOIN smelt.sql_l2_227 b ON a.user_id = b.user_id
