---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.referrer,
    b.user_id,
    c.is_verified,
    c.updated_at
FROM smelt.models.categories a
INNER JOIN smelt.models.categories b ON a.user_id = b.user_id
LEFT JOIN smelt.models.categories c ON a.user_id = c.user_id

