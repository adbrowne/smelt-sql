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
    MAX(created_at) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('sql_l1_58')
GROUP BY platform
HAVING COUNT(*) > 10
