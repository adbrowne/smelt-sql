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
    platform,
    browser,
    segment
FROM smelt.sql_l3_101
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_91 WHERE platform = 'web'
)

