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
    a.product_id,
    a.duration_seconds,
    b.referrer
FROM smelt.sql_l2_18 a
INNER JOIN smelt.sql_l2_147 b ON a.user_id = b.user_id
