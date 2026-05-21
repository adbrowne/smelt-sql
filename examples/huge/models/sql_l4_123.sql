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
    status,
    COUNT(DISTINCT user_id) AS val_1,
    AVG(price) AS val_2
FROM smelt.sql_l3_11
GROUP BY status
HAVING COUNT(*) > 10

