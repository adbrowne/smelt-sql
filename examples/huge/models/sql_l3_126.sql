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
    duration_seconds,
    platform,
    order_id
FROM smelt.sql_l2_200
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_31 WHERE is_active = true
)
