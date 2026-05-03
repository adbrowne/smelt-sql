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
    a.platform,
    b.browser
FROM smelt.products a
INNER JOIN smelt.products b ON a.user_id = b.user_id

