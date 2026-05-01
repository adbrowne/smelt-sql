---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_time,
    a.tier,
    b.page_path
FROM smelt.models.subscriptions a
INNER JOIN smelt.models.subscriptions b ON a.user_id = b.user_id

