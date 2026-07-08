---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    page_path,
    plan_type,
    browser
FROM smelt.sql_l1_100
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_100 WHERE score >= 50
)
