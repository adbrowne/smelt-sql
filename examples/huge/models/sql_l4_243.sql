---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    platform,
    AVG(price) AS val_1,
    COUNT(*) AS val_2
FROM smelt.models.sql_l3_31
GROUP BY platform
HAVING COUNT(*) > 10

