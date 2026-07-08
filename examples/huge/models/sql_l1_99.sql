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
    a.revenue,
    a.quantity,
    b.is_verified
FROM smelt.refunds a
LEFT JOIN smelt.refunds b ON a.user_id = b.user_id
