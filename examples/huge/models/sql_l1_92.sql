---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.is_verified,
    a.browser,
    b.discount
FROM smelt.models.errors a
LEFT JOIN smelt.models.errors b ON a.user_id = b.user_id

