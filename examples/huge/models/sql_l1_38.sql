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
    a.plan_type,
    b.category
FROM smelt.campaigns a
INNER JOIN smelt.campaigns b ON a.user_id = b.user_id

