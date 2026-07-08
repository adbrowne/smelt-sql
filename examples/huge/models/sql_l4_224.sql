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
    a.category,
    a.device_type,
    b.user_id
FROM smelt.sql_l3_17 a
INNER JOIN smelt.sql_l3_17 b ON a.user_id = b.user_id
