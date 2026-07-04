---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.duration_seconds,
    a.device_type,
    b.updated_at
FROM smelt.sql_l2_218 a
LEFT JOIN smelt.sql_l2_165 b ON a.user_id = b.user_id
