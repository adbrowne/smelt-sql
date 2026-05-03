---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_time,
    AVG(duration_seconds) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.sql_l3_91
GROUP BY event_time
HAVING COUNT(*) > 10

