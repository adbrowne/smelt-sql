---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    platform,
    AVG(amount) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.subscriptions
GROUP BY platform
HAVING COUNT(*) > 10
