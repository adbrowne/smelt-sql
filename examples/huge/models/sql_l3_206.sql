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
    quantity,
    score,
    event_type
FROM smelt.sql_l2_6
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_194 WHERE status = 'active'
)
