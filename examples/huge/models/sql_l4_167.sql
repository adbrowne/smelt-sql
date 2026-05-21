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
    quantity,
    SUM(quantity) AS val_1,
    AVG(price) AS val_2
FROM smelt.sql_l3_4
GROUP BY quantity
HAVING COUNT(*) > 10

