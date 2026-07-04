---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_time,
    MAX(created_at) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.refunds
GROUP BY event_time
HAVING COUNT(*) > 10
