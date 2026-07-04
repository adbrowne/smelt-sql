---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    price,
    is_verified,
    platform
FROM smelt.sql_l2_184
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_29 WHERE quantity > 0
)
