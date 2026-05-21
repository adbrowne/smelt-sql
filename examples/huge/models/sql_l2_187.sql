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
    cost,
    COUNT(*) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.sql_l1_179
GROUP BY cost
HAVING COUNT(*) > 10

