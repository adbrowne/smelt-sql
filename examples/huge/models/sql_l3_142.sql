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
    a.tier,
    b.profit,
    c.status,
    c.page_path
FROM smelt.sql_l2_144 a
INNER JOIN smelt.sql_l2_108 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_148 c ON a.user_id = c.user_id
