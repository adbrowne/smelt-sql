---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.discount,
    a.referrer,
    b.user_id
FROM smelt.orders a
LEFT JOIN smelt.orders b ON a.user_id = b.user_id
