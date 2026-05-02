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
    device_type,
    is_active
FROM smelt.models.sessions
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sessions WHERE platform = 'web'
)

