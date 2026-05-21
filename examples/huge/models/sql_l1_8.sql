---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    browser,
    updated_at,
    quantity
FROM smelt.errors
WHERE user_id IN (
    SELECT user_id FROM smelt.errors WHERE is_active = true
)

