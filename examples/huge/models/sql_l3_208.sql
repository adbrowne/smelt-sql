---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.created_at,
    b.os_name,
    c.user_id,
    c.score
FROM smelt.sql_l2_102 a
INNER JOIN smelt.sql_l2_210 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_102 c ON a.user_id = c.user_id
