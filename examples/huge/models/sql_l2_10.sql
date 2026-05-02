---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    discount,
    AVG(duration_seconds) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.models.sql_l1_204
GROUP BY discount
HAVING COUNT(*) > 10

