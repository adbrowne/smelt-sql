---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.revenue,
    b.updated_at,
    c.cohort_date,
    c.order_id
FROM smelt.sessions a
INNER JOIN smelt.sessions b ON a.user_id = b.user_id
LEFT JOIN smelt.sessions c ON a.user_id = c.user_id
