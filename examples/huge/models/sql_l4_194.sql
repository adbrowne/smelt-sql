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
    page_path,
    region
FROM smelt.sql_l3_155
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_165 WHERE platform = 'web'
)
