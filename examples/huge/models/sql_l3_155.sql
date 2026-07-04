---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    platform,
    SUM(revenue) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.sql_l2_241
GROUP BY platform
HAVING COUNT(*) > 10
