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
    os_name,
    AVG(duration_seconds) AS val_1,
    COUNT(*) AS val_2
FROM smelt.sql_l1_149
GROUP BY os_name
HAVING COUNT(*) > 10
