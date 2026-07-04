---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_type,
    quantity,
    country
FROM smelt.sql_l2_121
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_219 WHERE event_type = 'purchase'
)
