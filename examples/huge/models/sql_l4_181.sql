---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    AVG(amount) AS val_1,
    SUM(amount) AS val_2
FROM smelt.sql_l3_152
GROUP BY event_date
HAVING COUNT(*) > 10

