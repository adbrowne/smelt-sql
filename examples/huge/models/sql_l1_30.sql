---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.device_type,
    a.created_at,
    b.discount
FROM smelt.subscriptions a
LEFT JOIN smelt.subscriptions b ON a.user_id = b.user_id

