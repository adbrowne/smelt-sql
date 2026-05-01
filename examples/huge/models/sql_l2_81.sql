---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_type,
    AVG(amount) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.models.sql_l1_88
GROUP BY event_type
HAVING COUNT(*) > 10

