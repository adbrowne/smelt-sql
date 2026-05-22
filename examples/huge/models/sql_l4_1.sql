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
    session_id,
    is_verified,
    discount
FROM smelt.sql_l3_8
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_8 WHERE created_at >= '2024-01-01'
)
