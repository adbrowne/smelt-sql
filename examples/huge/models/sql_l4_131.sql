---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    duration_seconds,
    channel
FROM smelt.sql_l3_25
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_189 WHERE category IS NOT NULL
)
