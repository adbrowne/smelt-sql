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
    score,
    COUNT(*) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.sql_l3_159
GROUP BY score
HAVING COUNT(*) > 10
