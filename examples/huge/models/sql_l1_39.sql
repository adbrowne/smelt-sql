---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    device_type,
    is_active
FROM smelt.ref('sessions')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sessions') WHERE platform = 'web'
)
