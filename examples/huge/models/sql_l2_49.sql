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
    page_path,
    event_type,
    platform
FROM smelt.sql_l1_35
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_166 WHERE category IS NOT NULL
)
