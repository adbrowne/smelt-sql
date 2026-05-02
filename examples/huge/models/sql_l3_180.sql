---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    region,
    COUNT(*) AS val_1,
    AVG(amount) AS val_2
FROM smelt.models.sql_l2_56
GROUP BY region
HAVING COUNT(*) > 10

