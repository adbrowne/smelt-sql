---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    tier,
    AVG(duration_seconds) AS val_1,
    SUM(amount) AS val_2
FROM smelt.sql_l1_37
GROUP BY tier
HAVING COUNT(*) > 10
