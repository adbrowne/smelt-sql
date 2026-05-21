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
    cohort_date,
    MIN(created_at) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.sql_l3_212
GROUP BY cohort_date
HAVING COUNT(*) > 10

