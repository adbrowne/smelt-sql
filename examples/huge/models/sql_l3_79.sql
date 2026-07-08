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
    a.is_verified,
    a.duration_seconds,
    b.cost
FROM smelt.sql_l2_69 a
LEFT JOIN smelt.sql_l2_67 b ON a.user_id = b.user_id
