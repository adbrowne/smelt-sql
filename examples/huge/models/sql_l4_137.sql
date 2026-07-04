---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    MAX(created_at) AS val_1,
    AVG(price) AS val_2
FROM smelt.sql_l3_36
GROUP BY status
HAVING COUNT(*) > 10
