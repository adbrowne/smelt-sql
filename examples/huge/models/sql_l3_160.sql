---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    platform,
    AVG(duration_seconds) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.sql_l2_225
GROUP BY platform
HAVING COUNT(*) > 10
