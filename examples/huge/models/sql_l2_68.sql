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
    category,
    browser
FROM smelt.sql_l1_241
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_130 WHERE category IS NOT NULL
)

