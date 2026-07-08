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
    event_type,
    browser,
    amount
FROM smelt.sql_l2_106
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_220 WHERE category IS NOT NULL
)
