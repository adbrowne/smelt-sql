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
    event_date,
    COUNT(*) AS val_1,
    AVG(price) AS val_2
FROM smelt.sessions
GROUP BY event_date
HAVING COUNT(*) > 10
