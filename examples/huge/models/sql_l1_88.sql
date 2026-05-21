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
    a.event_time,
    a.tier,
    b.page_path
FROM smelt.subscriptions a
INNER JOIN smelt.subscriptions b ON a.user_id = b.user_id

