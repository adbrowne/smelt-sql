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
    a.revenue,
    a.category,
    b.is_active
FROM smelt.sql_l2_7 a
INNER JOIN smelt.sql_l2_245 b ON a.user_id = b.user_id

