---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    plan_type,
    platform,
    score
FROM smelt.ref('shipments')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('shipments') WHERE status = 'active'
)
