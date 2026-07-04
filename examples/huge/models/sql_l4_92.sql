---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    country,
    region,
    category,
    is_verified
FROM smelt.sql_l3_176
WHERE score >= 50
