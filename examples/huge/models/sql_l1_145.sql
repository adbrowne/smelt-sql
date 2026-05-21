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
    a.score,
    a.plan_type,
    b.region
FROM smelt.events a
LEFT JOIN smelt.events b ON a.user_id = b.user_id

