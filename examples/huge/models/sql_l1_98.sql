---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.discount,
    a.quantity,
    b.created_at
FROM smelt.clicks a
LEFT JOIN smelt.clicks b ON a.user_id = b.user_id

