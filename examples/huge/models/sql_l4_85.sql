---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    plan_type,
    AVG(duration_seconds) AS val_1,
    COUNT(*) AS val_2
FROM smelt.sql_l3_245
GROUP BY plan_type
HAVING COUNT(*) > 10
