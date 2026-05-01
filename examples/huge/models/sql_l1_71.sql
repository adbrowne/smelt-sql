---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.updated_at,
    b.is_verified,
    c.campaign_id,
    c.plan_type
FROM smelt.models.reviews a
INNER JOIN smelt.models.reviews b ON a.user_id = b.user_id
LEFT JOIN smelt.models.reviews c ON a.user_id = c.user_id

