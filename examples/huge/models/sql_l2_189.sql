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
    referrer,
    device_type,
    score
FROM smelt.sql_l1_46
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_206 WHERE amount > 0
)
