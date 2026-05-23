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
    b.os_name,
    c.amount,
    c.event_time
FROM smelt.sql_l1_78 a
INNER JOIN smelt.sql_l1_0 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_78 c ON a.user_id = c.user_id
