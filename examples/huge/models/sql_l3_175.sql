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
    profit,
    MAX(created_at) AS val_1,
    SUM(amount) AS val_2
FROM smelt.sql_l2_67
GROUP BY profit
HAVING COUNT(*) > 10

