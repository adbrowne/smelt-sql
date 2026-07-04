---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    rating,
    created_at,
    cost,
    event_date
FROM smelt.sql_l1_109
WHERE score >= 50
