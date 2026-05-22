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
    plan_type,
    category
FROM smelt.sql_l2_130
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_228 WHERE status = 'active'
)
