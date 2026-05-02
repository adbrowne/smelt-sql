---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    device_type,
    SUM(revenue) AS val_1,
    COUNT(*) AS val_2
FROM smelt.models.sql_l3_143
GROUP BY device_type
HAVING COUNT(*) > 10

