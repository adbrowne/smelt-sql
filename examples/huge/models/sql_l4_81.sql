---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_verified,
    event_type,
    category
FROM smelt.sql_l3_95
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_17 WHERE is_active = true
)
