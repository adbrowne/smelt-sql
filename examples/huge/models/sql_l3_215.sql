---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    created_at,
    MAX(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.models.sql_l2_249
GROUP BY created_at
HAVING COUNT(*) > 10

