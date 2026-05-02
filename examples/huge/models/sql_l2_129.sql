---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_active,
    AVG(duration_seconds) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.models.sql_l1_143
GROUP BY is_active
HAVING COUNT(*) > 10

