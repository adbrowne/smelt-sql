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
    segment,
    duration_seconds,
    region
FROM smelt.sql_l3_237
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_237 WHERE event_type = 'purchase'
)

