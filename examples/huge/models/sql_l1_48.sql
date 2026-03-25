---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    platform,
    score
FROM smelt.ref('shipments')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('shipments') WHERE status = 'active'
)
