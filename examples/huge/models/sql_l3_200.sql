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
    a.status,
    a.rating,
    b.updated_at
FROM smelt.sql_l2_56 a
LEFT JOIN smelt.sql_l2_37 b ON a.user_id = b.user_id
