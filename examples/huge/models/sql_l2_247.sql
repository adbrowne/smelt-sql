---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cost,
    AVG(price) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.models.sql_l1_12
GROUP BY cost
HAVING COUNT(*) > 10

