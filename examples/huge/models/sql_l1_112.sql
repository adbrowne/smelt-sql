---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.plan_type,
    b.revenue,
    c.tier,
    c.campaign_id
FROM smelt.models.logs a
INNER JOIN smelt.models.logs b ON a.user_id = b.user_id
LEFT JOIN smelt.models.logs c ON a.user_id = c.user_id

