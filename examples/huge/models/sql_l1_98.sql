---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.discount,
    a.quantity,
    b.created_at
FROM smelt.models.clicks a
LEFT JOIN smelt.models.clicks b ON a.user_id = b.user_id

