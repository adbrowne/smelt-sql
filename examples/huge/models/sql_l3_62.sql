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
    a.score,
    b.event_type,
    c.status,
    c.country
FROM smelt.sql_l2_184 a
INNER JOIN smelt.sql_l2_204 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_145 c ON a.user_id = c.user_id
