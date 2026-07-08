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
    a.amount,
    b.region,
    c.revenue,
    c.event_type
FROM smelt.sql_l1_127 a
INNER JOIN smelt.sql_l1_181 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_73 c ON a.user_id = c.user_id
