---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cost,
    AVG(duration_seconds) AS val_1,
    AVG(amount) AS val_2
FROM smelt.sql_l3_159
GROUP BY cost
HAVING COUNT(*) > 10
