---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cost,
    segment,
    price,
    score
FROM smelt.sql_l3_60
WHERE score >= 50
