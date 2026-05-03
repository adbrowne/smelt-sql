---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_time,
    MAX(created_at) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.sql_l1_58
GROUP BY event_time
HAVING COUNT(*) > 10

