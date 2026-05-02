---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    revenue,
    MIN(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.models.sql_l1_121
GROUP BY revenue
HAVING COUNT(*) > 10

