---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    device_type,
    SUM(amount) AS val_1,
    COUNT(*) AS val_2
FROM smelt.categories
GROUP BY device_type
HAVING COUNT(*) > 10
