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
    created_at,
    event_time,
    session_id
FROM smelt.sql_l3_52
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_151 WHERE quantity > 0
)
