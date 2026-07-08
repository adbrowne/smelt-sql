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
    a.os_name,
    a.country,
    b.quantity
FROM smelt.sql_l2_16 a
LEFT JOIN smelt.sql_l2_52 b ON a.user_id = b.user_id
