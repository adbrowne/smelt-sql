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
    AVG(price) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.sql_l2_158
GROUP BY status
HAVING COUNT(*) > 10

