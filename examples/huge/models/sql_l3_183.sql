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
    SUM(quantity) AS val_1,
    AVG(amount) AS val_2
FROM smelt.sql_l2_79
GROUP BY cost
HAVING COUNT(*) > 10

