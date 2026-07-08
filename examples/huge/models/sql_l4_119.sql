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
    a.event_date,
    b.event_time,
    c.score,
    c.channel
FROM smelt.sql_l3_148 a
INNER JOIN smelt.sql_l3_156 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_148 c ON a.user_id = c.user_id
