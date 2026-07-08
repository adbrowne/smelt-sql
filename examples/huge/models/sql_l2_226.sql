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
    a.is_active,
    b.platform,
    c.event_time,
    c.referrer
FROM smelt.sql_l1_241 a
INNER JOIN smelt.sql_l1_213 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_119 c ON a.user_id = c.user_id
