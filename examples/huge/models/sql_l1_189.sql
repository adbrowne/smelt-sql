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
    is_active,
    MIN(created_at) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.clicks
GROUP BY is_active
HAVING COUNT(*) > 10
