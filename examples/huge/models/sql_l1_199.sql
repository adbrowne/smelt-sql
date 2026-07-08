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
    a.event_type,
    b.status
FROM smelt.payments a
LEFT JOIN smelt.payments b ON a.user_id = b.user_id
