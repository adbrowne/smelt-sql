---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    revenue,
    status,
    created_at
FROM smelt.sql_l3_23
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_141 WHERE quantity > 0
)
