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
    status,
    device_type,
    session_id
FROM smelt.sql_l1_164
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_164 WHERE category IS NOT NULL
)
