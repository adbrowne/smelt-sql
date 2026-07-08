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
    channel,
    status,
    browser
FROM smelt.sql_l2_60
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_203 WHERE status = 'active'
)
