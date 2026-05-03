---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_active,
    order_id,
    duration_seconds,
    revenue
FROM smelt.notifications
WHERE category IS NOT NULL

