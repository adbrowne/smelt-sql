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
    b.quantity,
    c.os_name,
    c.browser
FROM smelt.models.subscriptions a
INNER JOIN smelt.models.subscriptions b ON a.user_id = b.user_id
LEFT JOIN smelt.models.subscriptions c ON a.user_id = c.user_id

