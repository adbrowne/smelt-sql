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
    revenue,
    updated_at,
    discount
FROM smelt.users
WHERE user_id IN (
    SELECT user_id FROM smelt.users WHERE event_type = 'purchase'
)
