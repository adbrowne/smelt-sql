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
    MAX(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.sql_l2_119
GROUP BY platform
HAVING COUNT(*) > 10

