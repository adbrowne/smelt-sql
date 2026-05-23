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
    is_verified,
    category,
    created_at
FROM smelt.orders
WHERE user_id IN (
    SELECT user_id FROM smelt.orders WHERE created_at >= '2024-01-01'
)
