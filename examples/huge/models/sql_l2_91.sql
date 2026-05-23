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
    a.created_at,
    b.platform,
    c.event_date,
    c.user_id
FROM smelt.sql_l1_137 a
INNER JOIN smelt.sql_l1_137 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_137 c ON a.user_id = c.user_id
