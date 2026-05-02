---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.platform,
    b.is_verified,
    c.is_active,
    c.plan_type
FROM smelt.models.errors a
INNER JOIN smelt.models.errors b ON a.user_id = b.user_id
LEFT JOIN smelt.models.errors c ON a.user_id = c.user_id

