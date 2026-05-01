---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.is_active,
    a.event_type,
    b.status
FROM smelt.models.payments a
LEFT JOIN smelt.models.payments b ON a.user_id = b.user_id

