---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    discount,
    tier,
    created_at,
    rating
FROM smelt.sql_l1_213
WHERE score >= 50
