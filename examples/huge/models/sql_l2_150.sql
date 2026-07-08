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
    a.region,
    b.browser
FROM smelt.sql_l1_240 a
INNER JOIN smelt.sql_l1_139 b ON a.user_id = b.user_id
