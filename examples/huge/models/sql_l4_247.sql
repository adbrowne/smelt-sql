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
    country,
    category,
    browser
FROM smelt.sql_l3_200
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_23 WHERE is_active = true
)
