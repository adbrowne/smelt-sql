---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    browser,
    plan_type,
    event_type
FROM smelt.ref('signups')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('signups') WHERE is_active = true
)
