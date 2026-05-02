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
    updated_at,
    quantity
FROM smelt.models.errors
WHERE user_id IN (
    SELECT user_id FROM smelt.models.errors WHERE is_active = true
)

