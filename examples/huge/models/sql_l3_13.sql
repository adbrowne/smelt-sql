---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_type,
    MAX(created_at) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.sql_l2_6
GROUP BY event_type
HAVING COUNT(*) > 10

