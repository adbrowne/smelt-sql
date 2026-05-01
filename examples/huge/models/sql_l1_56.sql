---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.revenue,
    b.price,
    c.country,
    c.page_path
FROM smelt.models.users a
INNER JOIN smelt.models.users b ON a.user_id = b.user_id
LEFT JOIN smelt.models.users c ON a.user_id = c.user_id

