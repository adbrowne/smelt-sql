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
    AVG(duration_seconds) AS val_1,
    MAX(created_at) AS val_2
FROM smelt.sql_l3_246
GROUP BY event_type
HAVING COUNT(*) > 10
