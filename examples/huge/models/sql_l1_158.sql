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
    b.is_active,
    c.cohort_date,
    c.session_id
FROM smelt.refunds a
INNER JOIN smelt.refunds b ON a.user_id = b.user_id
LEFT JOIN smelt.refunds c ON a.user_id = c.user_id
