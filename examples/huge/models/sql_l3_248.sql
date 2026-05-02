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
    MIN(created_at) AS val_1,
    MAX(created_at) AS val_2
FROM smelt.models.sql_l2_90
GROUP BY device_type
HAVING COUNT(*) > 10

