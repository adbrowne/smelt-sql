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
    a.platform,
    a.updated_at,
    b.os_name
FROM smelt.sql_l1_146 a
LEFT JOIN smelt.sql_l1_133 b ON a.user_id = b.user_id

